#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
UI_DIR="$ROOT_DIR/karhutla_app/apps/karhutla-ui"

if ! command -v dx >/dev/null 2>&1; then
  echo "Galat: Dioxus CLI ('dx') belum terpasang. Pasang dengan: cargo install dioxus-cli@0.7.10 --locked" >&2
  exit 1
fi

if ! rustup target list --installed | grep -q '^aarch64-linux-android$'; then
  echo "Menambahkan target Rust aarch64-linux-android..."
  rustup target add aarch64-linux-android
fi

if [[ -z "${ANDROID_HOME:-}" ]]; then
  if [[ -d "${HOME}/Android/Sdk" ]]; then
    export ANDROID_HOME="${HOME}/Android/Sdk"
  else
    echo "Galat: ANDROID_HOME belum diatur dan ${HOME}/Android/Sdk tidak ditemukan." >&2
    echo "Atur ANDROID_HOME & ANDROID_NDK_HOME terlebih dahulu, atau gunakan GitHub Actions CI (.github/workflows/build-apps.yml)." >&2
    exit 1
  fi
fi

if [[ -z "${ANDROID_NDK_HOME:-}" ]]; then
  NDK_CANDIDATE="$(find "${ANDROID_HOME}/ndk" -maxdepth 1 -mindepth 1 -type d 2>/dev/null | sort -V | tail -n 1 || true)"
  if [[ -n "$NDK_CANDIDATE" ]]; then
    export ANDROID_NDK_HOME="$NDK_CANDIDATE"
  else
    echo "Galat: ANDROID_NDK_HOME belum diatur dan NDK tidak ditemukan di ${ANDROID_HOME}/ndk." >&2
    exit 1
  fi
fi

NDK_BIN="${ANDROID_NDK_HOME}/toolchains/llvm/prebuilt/linux-x86_64/bin"
export NDK_HOME="${ANDROID_NDK_HOME}"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="${CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS:-} -C link-arg=-Wl,-z,max-page-size=16384"
if [[ -d "$NDK_BIN" ]]; then
  export PATH="${NDK_BIN}:${PATH}"
  export CC_aarch64_linux_android="${NDK_BIN}/aarch64-linux-android24-clang"
  export AR_aarch64_linux_android="${NDK_BIN}/llvm-ar"
fi

echo "==> Memeriksa kompilasi lintas platform aarch64-linux-android (mobile)..."
(cd "${ROOT_DIR}/karhutla_app" && cargo check -p karhutla-ui --target aarch64-linux-android --no-default-features --features mobile)

echo "==> Memaketkan Android APK (arm64-v8a) rilis lewat Dioxus CLI..."
(cd "${UI_DIR}" && dx bundle --platform android --target aarch64-linux-android --package-types apk --release)

echo "==> Selesai! Berkas APK tersedia di:"
find "${ROOT_DIR}/karhutla_app/target/dx/karhutla-ui" -type f -name '*.apk' 2>/dev/null || true
