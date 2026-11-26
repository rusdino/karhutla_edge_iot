//! Custom MQTT 3.1.1 QoS0 client with LWT over `embassy-net` TCP.
use core::fmt::{self, Write as _};
use core::sync::atomic::Ordering;

use embassy_futures::select::{select4, Either4};
use embassy_net::{tcp::TcpSocket, IpEndpoint, Ipv4Address, Stack};
use embassy_time::{with_timeout, Duration, Instant, Ticker, Timer};
use karhutla_core::{
    cfg,
    mqtt_codec::{
        decode_remaining_length, encode_connect_with_lwt, encode_publish, encode_subscribe,
        parse_publish,
    },
    record::{LOG_CAPACITY, RECORD_SIZE},
    CmdMsg,
};

use crate::{
    LogReq, LogResp, CMD_CH, LOG_REQ_CH, LOG_RESP_CH, MQTT_CONNECTED, SHARED_STATUS, TEL_SIG,
    TIME_SYNC_SIG,
};

pub const T_TIME_SYNC: &str = "karhutla/time/sync";
pub const T_LOG_REQ: &str = "karhutla/log/req";
pub const T_LOG_META: &str = "karhutla/log/meta";
pub const T_LOG_CHUNK: &str = "karhutla/log/chunk";
pub const T_LOG_ACK: &str = "karhutla/log/ack";
#[cfg(feature = "riil")]
pub const T_LOG_PCT: &str = "karhutla/t/log_pct";
#[cfg(feature = "riil")]
pub const T_LOG_FULL: &str = "karhutla/t/log_full";

pub const RX_BUF_LEN: usize = 4096;
pub const TX_BUF_LEN: usize = 4096;
const FRAME_BUF: usize = 512;
const PKT_BUF: usize = 1800;
const JS_BUF: usize = 1600;
const KEEPALIVE_S: u16 = 30;
const PING_EVERY: Duration = Duration::from_secs(10);

struct BufW<'a> {
    b: &'a mut [u8],
    n: usize,
}

impl fmt::Write for BufW<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let end = self.n + s.len();
        if end > self.b.len() {
            return Err(fmt::Error);
        }
        self.b[self.n..end].copy_from_slice(s.as_bytes());
        self.n = end;
        Ok(())
    }
}

fn fmt_into<'a>(buf: &'a mut [u8], args: fmt::Arguments<'_>) -> Result<&'a [u8], ()> {
    let n = {
        let mut w = BufW { b: &mut buf[..], n: 0 };
        w.write_fmt(args).map_err(|_| ())?;
        w.n
    };
    Ok(&buf[..n])
}

struct Out<'a, 's> {
    sock: &'a mut TcpSocket<'s>,
    pkt: [u8; PKT_BUF],
}

impl Out<'_, '_> {
    async fn send_pkt(&mut self, n: usize) -> Result<(), ()> {
        let mut off = 0;
        while off < n {
            let w = self.sock.write(&self.pkt[off..n]).await.map_err(|_| ())?;
            if w == 0 {
                return Err(());
            }
            off += w;
        }
        Ok(())
    }

    async fn publish(&mut self, topic: &str, payload: &[u8], retain: bool) -> Result<(), ()> {
        let n = encode_publish(&mut self.pkt, topic, payload, retain)?;
        self.send_pkt(n).await
    }

    async fn subscribe(&mut self, id: u16, topic: &str) -> Result<(), ()> {
        let n = encode_subscribe(&mut self.pkt, id, topic)?;
        self.send_pkt(n).await
    }

    async fn ping(&mut self) -> Result<(), ()> {
        self.pkt[..2].copy_from_slice(&[0xC0, 0x00]);
        self.send_pkt(2).await
    }
}

fn parse_req(p: &str) -> Option<LogReq> {
    let p = p.trim();
    if p.eq_ignore_ascii_case("META") {
        return Some(LogReq::Meta);
    }
    let mut it = p.split(':');
    match it.next()? {
        "GET" => {
            let start = it.next()?.parse().ok()?;
            let count = it.next()?.parse().ok()?;
            Some(LogReq::Get { start, count })
        }
        "CLEAR" => {
            let count = it.next()?.parse().ok()?;
            let crc = it.next()?.parse().ok()?;
            Some(LogReq::Clear { expected_count: count, expected_crc: crc })
        }
        _ => None,
    }
}

fn handle_publish(topic: &str, payload: &[u8]) {
    let Ok(s) = core::str::from_utf8(payload) else { return };
    let s = s.trim();
    if topic == cfg::T_CMD {
        let c = match s {
            "AUTO" => CmdMsg::Auto,
            "FORCE_MAX" => CmdMsg::ForceMax,
            "FORCE_OFF" => CmdMsg::ForceOff,
            "RESET" => CmdMsg::Reset,
            _ => return,
        };
        let _ = CMD_CH.try_send(c);
    } else if topic == T_TIME_SYNC {
        if let Ok(e) = s.parse::<u32>() {
            TIME_SYNC_SIG.signal(e);
        }
    } else if topic == T_LOG_REQ {
        if let Some(r) = parse_req(s) {
            let _ = LOG_REQ_CH.try_send(r);
        }
    } else {
        #[cfg(feature = "hils")]
        if topic == cfg::T_HILS_IN {
            let mut it = s.split(',');
            let seq = it.next().and_then(|x| x.trim().parse::<u32>().ok());
            let vd = it.next().and_then(|x| x.trim().parse::<f32>().ok());
            let vm = it.next().and_then(|x| x.trim().parse::<f32>().ok());
            if let (Some(seq), Some(vd), Some(vm)) = (seq, vd, vm) {
                if crate::HILS_CH.try_send((seq, vd, vm)).is_err() {
                    let _ = crate::HILS_CH.try_receive();
                    let _ = crate::HILS_CH.try_send((seq, vd, vm));
                }
            }
        }
    }
}

#[cfg(feature = "hils")]
const MODE_STR: &str = "HILS";
#[cfg(feature = "riil")]
const MODE_STR: &str = "RIIL";

async fn session(
    stack: Stack<'static>,
    rx_buf: &mut [u8],
    tx_buf: &mut [u8],
) -> Result<(), ()> {
    let mut sock = TcpSocket::new(stack, rx_buf, tx_buf);
    sock.set_timeout(Some(Duration::from_secs(30)));
    let host: Ipv4Address = {
        let mut o = [0u8; 4];
        for (i, part) in cfg::MQTT_HOST.split('.').enumerate().take(4) {
            o[i] = part.parse().map_err(|_| ())?;
        }
        Ipv4Address::new(o[0], o[1], o[2], o[3])
    };
    let ep = IpEndpoint::new(host.into(), cfg::MQTT_PORT);
    with_timeout(Duration::from_secs(10), sock.connect(ep))
        .await
        .map_err(|_| ())?
        .map_err(|_| ())?;

    let mut out = Out { sock: &mut sock, pkt: [0; PKT_BUF] };
    let n = encode_connect_with_lwt(&mut out.pkt, "karhutla-esp32", KEEPALIVE_S, cfg::T_STATUS, b"offline")?;
    out.send_pkt(n).await?;

    let mut rx = [0u8; FRAME_BUF];
    let mut have = 0usize;
    // CONNACK
    let connack = async {
        while have < 4 {
            let r = out.sock.read(&mut rx[have..]).await.map_err(|_| ())?;
            if r == 0 {
                return Err(());
            }
            have += r;
        }
        Ok(())
    };
    with_timeout(Duration::from_secs(5), connack).await.map_err(|_| ())??;
    if rx[0] != 0x20 || rx[3] != 0 {
        return Err(());
    }
    rx.copy_within(4..have, 0);
    have -= 4;

    out.publish(cfg::T_STATUS, b"online", true).await?;
    out.subscribe(1, cfg::T_CMD).await?;
    out.subscribe(2, T_TIME_SYNC).await?;
    out.subscribe(3, T_LOG_REQ).await?;
    #[cfg(feature = "hils")]
    out.subscribe(4, cfg::T_HILS_IN).await?;
    MQTT_CONNECTED.store(true, Ordering::Relaxed);

    let mut js = [0u8; JS_BUF];
    let mut hb = Ticker::every(Duration::from_secs(1));
    let mut last_ping = Instant::now();
    #[cfg(feature = "riil")]
    let mut last_scalar = Instant::from_millis(0);
    #[cfg(feature = "riil")]
    let mut prev: Option<(karhutla_core::AlarmState, karhutla_core::OpMode)> = None;

    loop {
        let ev = select4(out.sock.read(&mut rx[have..]), TEL_SIG.wait(), LOG_RESP_CH.receive(), hb.next()).await;
        match ev {
            Either4::First(r) => {
                let r = r.map_err(|_| ())?;
                if r == 0 {
                    return Err(());
                }
                have += r;
                loop {
                    if have < 2 {
                        break;
                    }
                    let Some((rem, lenb)) = decode_remaining_length(&rx[1..have]) else {
                        if have >= 5 {
                            return Err(());
                        }
                        break;
                    };
                    let total = 1 + lenb + rem;
                    if total > FRAME_BUF {
                        return Err(());
                    }
                    if have < total {
                        break;
                    }
                    let hdr = rx[0];
                    if let Some((topic, payload)) = parse_publish(hdr, &rx[1 + lenb..total]) {
                        handle_publish(topic, payload);
                    }
                    rx.copy_within(total..have, 0);
                    have -= total;
                }
            }
            Either4::Second(t) => {
                let b = fmt_into(
                    &mut js,
                    format_args!(
                        "{{\"seq\":{},\"v_dust\":{:.3},\"v_mq\":{:.3},\"pm_raw\":{:.2},\"pm\":{:.2},\"dcdt\":{:.3},\
\"gas_ppm\":{:.1},\"g\":{:.3},\"fi\":{:.2},\"v0\":{:.3},\"state\":\"{}\",\"mode\":\"{}\",\"duty\":{:.1},\"alarm\":{}}}",
                        t.seq, t.v_dust, t.v_mq, t.pm_raw, t.pm, t.dcdt, t.gas_ppm, t.g, t.fi, t.v0,
                        t.state.as_str(), t.mode.as_str(), t.duty, t.alarm
                    ),
                )?;
                out.publish(cfg::T_TELEM, b, false).await?;

                #[cfg(feature = "riil")]
                {
                    let now = Instant::now();
                    let changed = prev != Some((t.state, t.mode));
                    if changed || now.duration_since(last_scalar) >= Duration::from_millis(1000) {
                        last_scalar = now;
                        prev = Some((t.state, t.mode));
                        let mut v = [0u8; 32];
                        let b = fmt_into(&mut v, format_args!("{:.2}", t.pm))?;
                        out.publish("karhutla/t/pm", b, false).await?;
                        let b = fmt_into(&mut v, format_args!("{:.2}", t.pm_raw))?;
                        out.publish("karhutla/t/pm_raw", b, false).await?;
                        let b = fmt_into(&mut v, format_args!("{:.2}", t.fi))?;
                        out.publish("karhutla/t/fi", b, false).await?;
                        let b = fmt_into(&mut v, format_args!("{:.3}", t.dcdt))?;
                        out.publish("karhutla/t/dcdt", b, false).await?;
                        let b = fmt_into(&mut v, format_args!("{:.1}", t.gas_ppm))?;
                        out.publish("karhutla/t/gas_ppm", b, false).await?;
                        let b = fmt_into(&mut v, format_args!("{:.2}", t.g))?;
                        out.publish("karhutla/t/g", b, false).await?;
                        let b = fmt_into(&mut v, format_args!("{:.0}", t.duty))?;
                        out.publish("karhutla/t/duty", b, false).await?;
                        out.publish("karhutla/t/state", t.state.as_str().as_bytes(), false).await?;
                        out.publish("karhutla/t/mode", t.mode.as_str().as_bytes(), false).await?;
                    }
                }
            }
            Either4::Third(resp) => match resp {
                LogResp::Meta { meta, rtc_ok, epoch_s } => {
                    let b = fmt_into(
                        &mut js,
                        format_args!(
                            "{{\"count\":{},\"cap\":{},\"full\":{},\"used_bytes\":{},\"cap_bytes\":{},\"crc32\":{},\"rtc_ok\":{},\"epoch_s\":{}}}",
                            meta.count, meta.cap, meta.full, meta.used_bytes, meta.cap_bytes, meta.crc32, rtc_ok, epoch_s
                        ),
                    )?;
                    out.publish(T_LOG_META, b, false).await?;
                }
                LogResp::Chunk { start, count, total, len, data } => {
                    let data = core::str::from_utf8(&data[..len]).map_err(|_| ())?;
                    let b = fmt_into(
                        &mut js,
                        format_args!(
                            "{{\"start\":{},\"count\":{},\"total\":{},\"data\":\"{}\"}}",
                            start, count, total, data
                        ),
                    )?;
                    out.publish(T_LOG_CHUNK, b, false).await?;
                }
                LogResp::ClearStarted { sectors } => {
                    let b = fmt_into(
                        &mut js,
                        format_args!("{{\"action\":\"CLEAR\",\"status\":\"started\",\"sectors\":{}}}", sectors),
                    )?;
                    out.publish(T_LOG_ACK, b, false).await?;
                }
                LogResp::ClearDone { ok, reason, erased, count } => {
                    let b = if ok {
                        fmt_into(
                            &mut js,
                            format_args!(
                                "{{\"action\":\"CLEAR\",\"status\":\"done\",\"ok\":true,\"erased_sectors\":{},\"count\":{}}}",
                                erased, count
                            ),
                        )?
                    } else {
                        fmt_into(
                            &mut js,
                            format_args!("{{\"action\":\"CLEAR\",\"status\":\"done\",\"ok\":false,\"reason\":\"{}\"}}", reason),
                        )?
                    };
                    out.publish(T_LOG_ACK, b, false).await?;
                }
            },
            Either4::Fourth(()) => {
                let count = SHARED_STATUS.log_count.load(Ordering::Relaxed);
                let full = SHARED_STATUS.log_full.load(Ordering::Relaxed);
                let b = fmt_into(
                    &mut js,
                    format_args!(
                        "{{\"mode\":\"{}\",\"uptime_ms\":{},\"rtc_ok\":{},\"epoch_s\":{},\"log_count\":{},\"log_cap\":{},\"log_full\":{},\"log_bytes\":{},\"log_cap_bytes\":{}}}",
                        MODE_STR,
                        Instant::now().as_millis() as u32,
                        SHARED_STATUS.rtc_ok.load(Ordering::Relaxed),
                        SHARED_STATUS.epoch_s.load(Ordering::Relaxed),
                        count,
                        LOG_CAPACITY,
                        full,
                        count * RECORD_SIZE as u32,
                        LOG_CAPACITY * RECORD_SIZE as u32
                    ),
                )?;
                out.publish(cfg::T_HEARTBEAT, b, false).await?;
                #[cfg(feature = "riil")]
                {
                    let mut v = [0u8; 16];
                    let pct = count as f32 * 100.0 / LOG_CAPACITY as f32;
                    let b = fmt_into(&mut v, format_args!("{:.1}", pct))?;
                    out.publish(T_LOG_PCT, b, false).await?;
                    out.publish(T_LOG_FULL, if full { b"1" } else { b"0" }, false).await?;
                }
                if Instant::now().duration_since(last_ping) >= PING_EVERY {
                    out.ping().await?;
                    last_ping = Instant::now();
                }
            }
        }
    }
}

#[embassy_executor::task]
pub async fn mqtt_task(stack: Stack<'static>) {
    static RX_BUF: static_cell::StaticCell<[u8; RX_BUF_LEN]> = static_cell::StaticCell::new();
    static TX_BUF: static_cell::StaticCell<[u8; TX_BUF_LEN]> = static_cell::StaticCell::new();
    let rx = RX_BUF.init([0; RX_BUF_LEN]);
    let tx = TX_BUF.init([0; TX_BUF_LEN]);
    loop {
        stack.wait_config_up().await;
        let _ = session(stack, &mut rx[..], &mut tx[..]).await;
        MQTT_CONNECTED.store(false, Ordering::Relaxed);
        Timer::after_secs(2).await;
    }
}
