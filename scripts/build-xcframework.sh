#!/bin/bash
# Builds the Rust core for the macOS app as a local Swift package (git-ignored, rebuilt by this script):
#
#   apps/macos/Generated/AutopaperCore/
#     Package.swift                                library "AutopaperCore", macOS 26
#     AutopaperCoreFFI.xcframework/                libautopaper_core.a (arm64 + x86_64), C header, modulemap
#     Sources/AutopaperCore/autopaper_core.swift   UniFFI's Swift bindings (compiled in Swift 5 language mode)
#
# The app adds it as a local package (XcodeGen: `packages: AutopaperCore: path: Generated/AutopaperCore`) and
# imports AutopaperCore. Rerun after any change to the core.
#
# Usage: scripts/build-xcframework.sh [--smoke]
#   --smoke  then builds and runs a tiny Swift 6 executable against the package (in target/, temporary): it
#            opens an Engine on a temporary directory with the Demo provider, adds a keyword, makes one
#            wallpaper and prints its title. This proves the package links and runs.
#
# Toolchain choices (memory-bank/techContext.md, docs/research/rust-linux.md):
# - rustup's stable toolchain (`rustup run stable`, with its bin directory first on PATH): Homebrew's
#   cargo and rustc, first on PATH on the dev Mac, have no x86_64 standard library.
# - MACOSX_DEPLOYMENT_TARGET is unset for cargo/rustc: set during a release build it breaks host proc-macro
#   dylibs on macOS 27 + Xcode 27 ("mis-aligned LINKEDIT string pool"). Xcode applies the app's deployment
#   target at the final link. Xcode exports the variable to run-script phases, so it is unset explicitly.
# - C code built by cc-rs (aws-lc-sys, onig_sys, libsqlite3-sys) gets -mmacosx-version-min=26.0 through
#   CFLAGS_<target>, so it targets macOS 26 rather than the macOS of the machine building it.
# - Release profile (thin LTO, one codegen unit); only the static library is built (`--crate-type staticlib`).
# - The package's linkerSettings are what rustc says the static library needs (`--print native-static-libs`;
#   on 2026-10-05: Security, SystemConfiguration and CoreFoundation for rustls-platform-verifier and the
#   system proxy settings, and libiconv). libSystem, libc and libm come with every Swift link.
#
# Needs: rustup with the aarch64-apple-darwin and x86_64-apple-darwin targets; Xcode (xcodebuild, lipo,
# swift).

set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$REPO/target}"
DEST="$REPO/apps/macos/Generated/AutopaperCore"
WORK="$TARGET_DIR/autopaper-swift"
STAGE="$WORK/AutopaperCore"
HEADERS="$WORK/include"
UNIVERSAL="$TARGET_DIR/macos-universal/release/libautopaper_core.a"
TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
MIN_MACOS="26.0"

SMOKE=0
for arg in "$@"; do
    case "$arg" in
        --smoke) SMOKE=1 ;;
        -h | --help)
            sed -n '2,15p' "$0"
            exit 0
            ;;
        *)
            echo "build-xcframework: unknown option $arg (try --help)" >&2
            exit 2
            ;;
    esac
done

die() {
    echo "build-xcframework: $*" >&2
    exit 1
}

step() {
    printf '\n==> %s\n' "$*"
}

command -v rustup > /dev/null || die "rustup isn't installed (https://rustup.rs)"
for tool in xcodebuild lipo swift; do
    command -v "$tool" > /dev/null || die "$tool isn't available: install Xcode and run xcode-select"
done
installed="$(rustup target list --installed --toolchain stable)"
for target in "${TARGETS[@]}"; do
    grep -qx "$target" <<< "$installed" || die "the $target target is missing: rustup target add --toolchain stable $target"
done

cd "$REPO"
# `rustup run stable cargo` starts the toolchain's cargo, but cargo calls the first `rustc` on PATH: on a Mac
# whose rustup is Homebrew's (no proxies on PATH) that is Homebrew's rustc, which has no x86_64 standard
# library. The toolchain's own bin directory goes first, so cargo and rustc match.
TOOLCHAIN_BIN="$(dirname "$(rustup which --toolchain stable rustc)")"
export PATH="$TOOLCHAIN_BIN:$PATH"
echo "Rust: $(rustc --version) · $(cargo --version)"
unset MACOSX_DEPLOYMENT_TARGET
export CFLAGS_aarch64_apple_darwin="-mmacosx-version-min=$MIN_MACOS"
export CFLAGS_x86_64_apple_darwin="-mmacosx-version-min=$MIN_MACOS"

mkdir -p "$WORK"
NATIVE_LIBS=""
for target in "${TARGETS[@]}"; do
    step "Building the core for $target (release)"
    log="$WORK/build-$target.log"
    rustup run stable cargo rustc -p autopaper-core --lib --release --target "$target" --crate-type staticlib \
        -- --print native-static-libs 2>&1 | tee "$log"
    libs="$(sed -n 's/.*native-static-libs: //p' "$log" | tail -n 1)"
    [ -n "$libs" ] || die "rustc didn't say which native libraries $target needs (see $log)"
    NATIVE_LIBS="$NATIVE_LIBS $libs"
done

step "Combining arm64 and x86_64 into one static library"
mkdir -p "$(dirname "$UNIVERSAL")"
lipo -create -output "$UNIVERSAL" \
    "$TARGET_DIR/aarch64-apple-darwin/release/libautopaper_core.a" \
    "$TARGET_DIR/x86_64-apple-darwin/release/libautopaper_core.a"
lipo -info "$UNIVERSAL"

step "Generating the Swift bindings"
rustup run stable cargo build -q -p uniffi-bindgen --bin uniffi-bindgen-swift
BINDGEN="$TARGET_DIR/debug/uniffi-bindgen-swift"
# The metadata is the same in either architecture's library.
LIBRARY="$TARGET_DIR/aarch64-apple-darwin/release/libautopaper_core.a"
rm -rf "$STAGE" "$HEADERS"
mkdir -p "$STAGE/Sources/AutopaperCore" "$HEADERS/autopaper_coreFFI"
"$BINDGEN" "$LIBRARY" "$STAGE/Sources/AutopaperCore" --swift-sources
# Headers in a module-named folder, and no --xcframework (that writes `framework module`, which is for
# .framework bundles): docs/research/rust-linux.md.
"$BINDGEN" "$LIBRARY" "$HEADERS/autopaper_coreFFI" --headers --modulemap \
    --module-name autopaper_coreFFI --modulemap-filename module.modulemap

step "Creating AutopaperCoreFFI.xcframework"
xcodebuild -create-xcframework -library "$UNIVERSAL" -headers "$HEADERS" \
    -output "$STAGE/AutopaperCoreFFI.xcframework" > /dev/null

# linkerSettings from rustc's list (both architectures), without duplicates or what every link has.
linker_settings=""
linked=""
# Split into words on purpose (the list holds no glob characters).
# shellcheck disable=SC2086
set -- $NATIVE_LIBS
while [ $# -gt 0 ]; do
    case "$1" in
        -framework)
            [ $# -ge 2 ] || die "rustc's native library list ends in -framework"
            entry=".linkedFramework(\"$2\")"
            name="$2.framework"
            shift 2
            ;;
        -lSystem | -lc | -lm)
            shift
            continue
            ;;
        -l*)
            entry=".linkedLibrary(\"${1#-l}\")"
            name="lib${1#-l}"
            shift
            ;;
        *) die "unexpected entry in rustc's native library list: $1" ;;
    esac
    case "$linker_settings" in *"$entry"*) continue ;; esac
    linker_settings+="                $entry,
"
    linked="${linked:+$linked, }$name"
done
echo "Links: $linked"
cat > "$STAGE/Package.swift" << EOF
// swift-tools-version:6.2
// Generated by scripts/build-xcframework.sh; don't edit (rerun the script).
import PackageDescription

let package = Package(
    name: "AutopaperCore",
    platforms: [.macOS(.v26)],
    products: [.library(name: "AutopaperCore", targets: ["AutopaperCore"])],
    targets: [
        .binaryTarget(name: "AutopaperCoreFFI", path: "AutopaperCoreFFI.xcframework"),
        .target(
            name: "AutopaperCore",
            dependencies: ["AutopaperCoreFFI"],
            // UniFFI's generated async foreign-trait code doesn't compile in Swift 6 mode; the app stays on 6.
            swiftSettings: [.swiftLanguageMode(.v5)],
            // What rustc says the static library needs (--print native-static-libs).
            linkerSettings: [
$linker_settings            ]
        ),
    ]
)
EOF

step "Installing the package at ${DEST#"$REPO"/}"
rm -rf "$DEST"
mkdir -p "$(dirname "$DEST")"
mv "$STAGE" "$DEST"
rm -rf "$HEADERS"

if [ "$SMOKE" = 1 ]; then
    step "Smoke test: a Swift 6 executable linked against the package"
    SMOKE_DIR="$WORK/smoke"
    rm -rf "$SMOKE_DIR"
    mkdir -p "$SMOKE_DIR/Sources/AutopaperSmoke"
    cat > "$SMOKE_DIR/Package.swift" << EOF
// swift-tools-version:6.2
import PackageDescription

let package = Package(
    name: "AutopaperSmoke",
    platforms: [.macOS(.v26)],
    dependencies: [.package(path: "$DEST")],
    targets: [
        .executableTarget(name: "AutopaperSmoke", dependencies: [.product(name: "AutopaperCore", package: "AutopaperCore")]),
    ]
)
EOF
    cat > "$SMOKE_DIR/Sources/AutopaperSmoke/main.swift" << 'EOF'
import AutopaperCore
import Foundation
import Synchronization

/// Keys in memory (the Demo provider needs none).
final class MemorySecrets: SecretStore {
    private let values = Mutex<[String: String]>([:])
    func get(account: String) -> String? { values.withLock { $0[account] } }
    func set(account: String, value: String) { values.withLock { $0[account] = value } }
    func delete(account: String) { values.withLock { $0[account] = nil } }
}

let directory = FileManager.default.temporaryDirectory.appendingPathComponent("autopaper-smoke-\(UUID().uuidString)")
defer { try? FileManager.default.removeItem(at: directory) }
let engine = try Engine.open(
    config: EngineConfig(dataDir: directory.path, modelDir: "", locale: "en-US", client: "macOS AutoPaper-smoke"),
    secrets: MemorySecrets()
)
let settings = try engine.settings()
print("providers: \(settings.textProvider.kind) / \(settings.imageProvider.kind)")
_ = try engine.addKeyword(text: "lighthouse", weight: .must)
let generation = try await engine.generate(trigger: .manual, observer: nil)
print("title: \(generation.concept.title)")
print("image: \(generation.width)x\(generation.height), memory: \(engine.memoryStatus().embeddingModel)")
// Free functions and typed error reasons, as the apps use them.
print("prices as of \(pricesAsOf()); Ollama at \(defaultBaseUrl(kind: .ollama) ?? "-"); "
    + "OpenAI writes with \(defaultModel(kind: .openAi, job: .concepts)); 0.35 is \(surpriseBand(surprise: 0.35)); "
    + "echoes: \(try engine.hasEchoes(id: generation.id))")
do {
    try engine.setDisplayHint(width: 0, height: 1080)
} catch AutoPaperError.InvalidInput(let reason, _) {
    print("invalid input: \(reason)")
}
EOF
    (cd "$SMOKE_DIR" && swift run -c release AutopaperSmoke)
    rm -rf "$SMOKE_DIR"
fi

step "Done: ${DEST#"$REPO"/}"
