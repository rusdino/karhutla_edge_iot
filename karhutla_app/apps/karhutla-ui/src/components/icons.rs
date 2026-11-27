use dioxus::prelude::*;

#[component]
pub fn IcoFlame(#[props(default = "ico".to_string())] class: String) -> Element {
    rsx! {
        svg {
            class: "{class}",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            "aria-hidden": "true",
            path { d: "M12 2c.5 4-3 6-3 10a5 5 0 0 0 10 0c0-3-2-5.5-4-8" }
            path { d: "M9 14a3 3 0 0 0 6 0" }
        }
    }
}

#[component]
pub fn IcoSun() -> Element {
    rsx! {
        svg {
            class: "ico",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            "aria-hidden": "true",
            circle { cx: "12", cy: "12", r: "4" }
            path { d: "M12 2v2" }
            path { d: "M12 20v2" }
            path { d: "m4.93 4.93 1.41 1.41" }
            path { d: "m17.66 17.66 1.41 1.41" }
            path { d: "M2 12h2" }
            path { d: "M20 12h2" }
            path { d: "m6.34 17.66-1.41 1.41" }
            path { d: "m19.07 4.93-1.41 1.41" }
        }
    }
}

#[component]
pub fn IcoMoon() -> Element {
    rsx! {
        svg {
            class: "ico",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            "aria-hidden": "true",
            path { d: "M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" }
        }
    }
}

#[component]
pub fn IcoMonitor() -> Element {
    rsx! {
        svg {
            class: "ico",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            "aria-hidden": "true",
            rect { x: "2", y: "3", width: "20", height: "14", rx: "2" }
            line { x1: "8", y1: "21", x2: "16", y2: "21" }
            line { x1: "12", y1: "17", x2: "12", y2: "21" }
        }
    }
}

#[component]
pub fn IcoHelp() -> Element {
    rsx! {
        svg {
            class: "ico",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            "aria-hidden": "true",
            circle { cx: "12", cy: "12", r: "10" }
            path { d: "M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3" }
            line { x1: "12", y1: "17", x2: "12.01", y2: "17" }
        }
    }
}

#[component]
pub fn IcoServer() -> Element {
    rsx! {
        svg {
            class: "ico",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            "aria-hidden": "true",
            rect { x: "2", y: "2", width: "20", height: "8", rx: "2", ry: "2" }
            rect { x: "2", y: "14", width: "20", height: "8", rx: "2", ry: "2" }
            line { x1: "6", y1: "6", x2: "6.01", y2: "6" }
            line { x1: "6", y1: "18", x2: "6.01", y2: "18" }
        }
    }
}

#[component]
pub fn IcoAlert() -> Element {
    rsx! {
        svg {
            class: "ico",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            "aria-hidden": "true",
            path { d: "M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0Z" }
            line { x1: "12", y1: "9", x2: "12", y2: "13" }
            line { x1: "12", y1: "17", x2: "12.01", y2: "17" }
        }
    }
}

#[component]
pub fn IcoFsm(state: String) -> Element {
    match state.as_str() {
        "NORMAL" => rsx! {
            svg {
                class: "ico kpi-state-ico",
                view_box: "0 0 24 24",
                fill: "none",
                stroke: "currentColor",
                stroke_width: "2",
                stroke_linecap: "round",
                stroke_linejoin: "round",
                "aria-hidden": "true",
                path { d: "M22 11.08V12a10 10 0 1 1-5.93-9.14" }
                polyline { points: "22 4 12 14.01 9 11.01" }
            }
        },
        "WASPADA" => rsx! {
            svg {
                class: "ico kpi-state-ico",
                view_box: "0 0 24 24",
                fill: "none",
                stroke: "currentColor",
                stroke_width: "2",
                stroke_linecap: "round",
                stroke_linejoin: "round",
                "aria-hidden": "true",
                path { d: "M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0Z" }
                line { x1: "12", y1: "9", x2: "12", y2: "13" }
                line { x1: "12", y1: "17", x2: "12.01", y2: "17" }
            }
        },
        "BAHAYA" => rsx! { IcoFlame { class: "ico kpi-state-ico".to_string() } },
        "FAILSAFE" => rsx! {
            svg {
                class: "ico kpi-state-ico",
                view_box: "0 0 24 24",
                fill: "none",
                stroke: "currentColor",
                stroke_width: "2",
                stroke_linecap: "round",
                stroke_linejoin: "round",
                "aria-hidden": "true",
                circle { cx: "12", cy: "12", r: "9" }
                line { x1: "4.93", y1: "4.93", x2: "19.07", y2: "19.07" }
            }
        },
        _ => rsx! {
            svg {
                class: "ico kpi-state-ico",
                view_box: "0 0 24 24",
                fill: "none",
                stroke: "currentColor",
                stroke_width: "2",
                stroke_linecap: "round",
                stroke_linejoin: "round",
                "aria-hidden": "true",
                circle { cx: "12", cy: "12", r: "9" }
                line { x1: "8", y1: "12", x2: "16", y2: "12" }
            }
        },
    }
}

#[component]
pub fn IcoPulse() -> Element {
    rsx! {
        svg {
            class: "ico",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            "aria-hidden": "true",
            polyline { points: "22 12 18 12 15 21 9 3 6 12 2 12" }
        }
    }
}

#[component]
pub fn IcoCloud() -> Element {
    rsx! {
        svg {
            class: "ico",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            "aria-hidden": "true",
            path { d: "M4 14.899A7 7 0 1 1 15.71 8h1.79a4.5 4.5 0 0 1 2.5 8.242" }
            path { d: "M8 19h8" }
            path { d: "M6 22h10" }
        }
    }
}

#[component]
pub fn IcoFlask() -> Element {
    rsx! {
        svg {
            class: "ico",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            "aria-hidden": "true",
            path { d: "M10 2v7.527a2 2 0 0 1-.211.896L4.72 20.55a1 1 0 0 0 .9 1.45h12.76a1 1 0 0 0 .9-1.45l-5.069-10.127A2 2 0 0 1 14 9.527V2" }
            path { d: "M8.5 2h7" }
        }
    }
}

#[component]
pub fn IcoClock() -> Element {
    rsx! {
        svg {
            class: "ico",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            "aria-hidden": "true",
            circle { cx: "12", cy: "12", r: "9" }
            path { d: "M12 7v5l3 2" }
        }
    }
}

#[component]
pub fn IcoFan() -> Element {
    rsx! {
        svg {
            class: "ico",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            "aria-hidden": "true",
            path { d: "M12 12c-2-2.67-6-2.67-6 0a4 4 0 0 0 6 0Zm0 0c2.67-2 2.67-6 0-6a4 4 0 0 0 0 6Zm0 0c2 2.67 6 2.67 6 0a4 4 0 0 0-6 0Zm0 0c-2.67 2-2.67 6 0 6a4 4 0 0 0 0-6Z" }
        }
    }
}

#[component]
pub fn IcoDownload() -> Element {
    rsx! {
        svg {
            class: "ico",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            "aria-hidden": "true",
            path { d: "M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" }
            polyline { points: "7 10 12 15 17 10" }
            line { x1: "12", y1: "15", x2: "12", y2: "3" }
        }
    }
}

#[component]
pub fn IcoFile() -> Element {
    rsx! {
        svg {
            class: "ico",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            "aria-hidden": "true",
            path { d: "M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" }
            polyline { points: "14 2 14 8 20 8" }
            line { x1: "8", y1: "13", x2: "16", y2: "13" }
            line { x1: "8", y1: "17", x2: "16", y2: "17" }
        }
    }
}

#[component]
pub fn IcoSliders() -> Element {
    rsx! {
        svg {
            class: "ico",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            "aria-hidden": "true",
            line { x1: "4", y1: "21", x2: "4", y2: "14" }
            line { x1: "4", y1: "10", x2: "4", y2: "3" }
            line { x1: "12", y1: "21", x2: "12", y2: "12" }
            line { x1: "12", y1: "8", x2: "12", y2: "3" }
            line { x1: "20", y1: "21", x2: "20", y2: "16" }
            line { x1: "20", y1: "12", x2: "20", y2: "3" }
            line { x1: "1", y1: "14", x2: "7", y2: "14" }
            line { x1: "9", y1: "8", x2: "15", y2: "8" }
            line { x1: "17", y1: "16", x2: "23", y2: "16" }
        }
    }
}

#[component]
pub fn IcoPlay() -> Element {
    rsx! {
        svg {
            class: "ico",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            "aria-hidden": "true",
            polygon { points: "6 3 20 12 6 21 6 3" }
        }
    }
}

#[component]
pub fn IcoGear() -> Element {
    rsx! {
        svg {
            class: "ico",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            "aria-hidden": "true",
            circle { cx: "12", cy: "12", r: "3" }
            path { d: "M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9c.26.6.77 1.02 1.4 1.15.2.04.41.06.62.06H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1Z" }
        }
    }
}

#[component]
pub fn IcoBolt() -> Element {
    rsx! {
        svg {
            class: "ico",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            "aria-hidden": "true",
            polygon { points: "13 2 3 14 12 14 11 22 21 10 12 10 13 2" }
        }
    }
}

#[component]
pub fn IcoStopSquare() -> Element {
    rsx! {
        svg {
            class: "ico",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            "aria-hidden": "true",
            rect { x: "6", y: "6", width: "12", height: "12", rx: "2" }
        }
    }
}

#[component]
pub fn IcoReset() -> Element {
    rsx! {
        svg {
            class: "ico",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            "aria-hidden": "true",
            polyline { points: "1 4 1 10 7 10" }
            path { d: "M3.51 15a9 9 0 1 0 2.13-9.36L1 10" }
        }
    }
}

#[component]
pub fn IcoChevron() -> Element {
    rsx! {
        svg {
            class: "ico drawer-chev",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            "aria-hidden": "true",
            polyline { points: "6 9 12 15 18 9" }
        }
    }
}

#[component]
pub fn IcoPhone() -> Element {
    rsx! {
        svg {
            class: "ico",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            "aria-hidden": "true",
            rect { x: "5", y: "2", width: "14", height: "20", rx: "2", ry: "2" }
            line { x1: "12", y1: "18", x2: "12.01", y2: "18" }
        }
    }
}
