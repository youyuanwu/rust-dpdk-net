#!/bin/bash
# Check build/runtime dependencies for dpdk-net and print the commands to install
# anything that is missing.
#
# Usage: ./scripts/check-deps.sh [options]
#
# Options:
#   --source-build   Also check tools needed to build DPDK from source (CMakeLists.txt)
#   --runtime        Also check runtime prerequisites (hugepages, root access)
#   --fix            Run the suggested apt-get install command instead of only printing it
#   --verbose        Show diagnostic output from the trial link
#   -h, --help       Show this help
#
# Exit codes: 0 = all required dependencies present, 1 = something is missing.

set -u

SOURCE_BUILD=false
RUNTIME=false
FIX=false
VERBOSE=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --source-build) SOURCE_BUILD=true ;;
        --runtime) RUNTIME=true ;;
        --fix) FIX=true ;;
        --verbose|-v) VERBOSE=true ;;
        -h|--help)
            awk 'NR > 1 { if (!/^#/) exit; sub(/^# ?/, ""); print }' "${BASH_SOURCE[0]}"
            exit 0
            ;;
        *)
            echo "Unknown option: $1" >&2
            echo "Try '${BASH_SOURCE[0]} --help'." >&2
            exit 2
            ;;
    esac
    shift
done

if [[ -t 1 && -z "${NO_COLOR:-}" ]]; then
    C_OK=$'\033[32m'; C_BAD=$'\033[31m'; C_WARN=$'\033[33m'; C_DIM=$'\033[2m'; C_OFF=$'\033[0m'
else
    C_OK=""; C_BAD=""; C_WARN=""; C_DIM=""; C_OFF=""
fi

MISSING_PKGS=()
OPTIONAL_PKGS=()
FAILED=0
NOTES=()

ok()   { printf '  %s✔%s %s\n' "$C_OK" "$C_OFF" "$1"; }
bad()  { printf '  %s✘%s %s\n' "$C_BAD" "$C_OFF" "$1"; FAILED=1; }
warn() { printf '  %s!%s %s\n' "$C_WARN" "$C_OFF" "$1"; }
head_() { printf '\n%s\n' "$1"; }

# Record a missing package for the final install command.
need() { MISSING_PKGS+=("$1"); }
want() { OPTIONAL_PKGS+=("$1"); }

# True if the linker can resolve a static archive by exact filename.
have_archive() {
    local path
    path=$(${CC:-cc} -print-file-name="$1" 2>/dev/null)
    [[ "$path" != "$1" && -e "$path" ]]
}

check_cmd() {
    local cmd="$1" pkg="$2" required="$3" label="${4:-$1}"
    if command -v "$cmd" >/dev/null 2>&1; then
        ok "$label"
        return 0
    fi
    if [[ "$required" == required ]]; then
        bad "$label (missing) -> $pkg"
        need "$pkg"
    else
        warn "$label (missing, optional) -> $pkg"
        want "$pkg"
    fi
    return 1
}

# Map a linker library name to the Debian/Ubuntu package that provides it.
pkg_for_lib() {
    case "$1" in
        jitterentropy)    echo libjitterentropy3-dev ;;
        rte_*)            echo libdpdk-dev ;;
        bsd)              echo libbsd-dev ;;
        md)               echo libmd-dev ;;
        numa)             echo libnuma-dev ;;
        fdt)              echo libfdt-dev ;;
        pcap)             echo libpcap-dev ;;
        xdp)              echo libxdp-dev ;;
        bpf)              echo libbpf-dev ;;
        jansson)          echo libjansson-dev ;;
        elf)              echo libelf-dev ;;
        zstd)             echo libzstd-dev ;;
        z)                echo zlib1g-dev ;;
        isal)             echo libisal-dev ;;
        IPSec_MB)         echo libipsec-mb-dev ;;
        crypto|ssl)       echo libssl-dev ;;
        dbus-1)           echo libdbus-1-dev ;;
        systemd)          echo libsystemd-dev ;;
        mnl)              echo libmnl-dev ;;
        nl-3|nl-route-3)  echo libnl-3-dev ;;
        ibverbs|mlx5|mlx4|mana|rdmacm|*-rdmav*|efa|hns|ionic) echo libibverbs-dev ;;
        *)                echo "" ;;
    esac
}

echo "dpdk-net dependency check"
if [[ -r /etc/os-release ]]; then
    # shellcheck disable=SC1091
    printf '%s%s%s\n' "$C_DIM" "$(. /etc/os-release && echo "$PRETTY_NAME")" "$C_OFF"
fi

head_ "Toolchain"
check_cmd cargo rustup required "cargo"
check_cmd cc build-essential required "cc (C compiler)"
check_cmd pkg-config pkg-config required "pkg-config"

head_ "DPDK"
DPDK_OK=false
if pkg-config --exists libdpdk 2>/dev/null; then
    ok "libdpdk $(pkg-config --modversion libdpdk) (pkg-config)"
    DPDK_OK=true
else
    bad "libdpdk not found by pkg-config -> libdpdk-dev"
    need libdpdk-dev
    NOTES+=("If you installed DPDK to a custom prefix, export PKG_CONFIG_PATH to its pkgconfig directory.")
fi

head_ "Static link dependencies"
# dpdk-net-sys reads DPDK's static pkg-config metadata, so it pulls in every
# Libs.private entry of libdpdk.pc and its transitive Requires.private.
if have_archive libjitterentropy.a; then
    ok "libjitterentropy.a"
else
    bad "libjitterentropy.a (missing) -> libjitterentropy3-dev"
    need libjitterentropy3-dev
    NOTES+=("libssl-dev's libcrypto.pc lists '-l:libjitterentropy.a' in Libs.private but does not depend on the package providing it.")
fi

# Trial link: the authoritative check that `cargo build` will succeed.
if [[ "$DPDK_OK" == true ]] && command -v cc >/dev/null 2>&1; then
    PROBE_DIR=$(mktemp -d)
    trap 'rm -rf "$PROBE_DIR"' EXIT
    cat >"$PROBE_DIR/probe.c" <<'EOF'
#include <rte_eal.h>
int main(int argc, char **argv) { return rte_eal_init(argc, argv); }
EOF
    # Mirror build.rs: the pkgconf crate rewrites `-l:libfoo.a` into `-lfoo`.
    read -r -a PROBE_CFLAGS <<<"$(pkg-config --static --cflags libdpdk 2>/dev/null)"
    read -r -a PROBE_LIBS <<<"$(pkg-config --static --libs libdpdk 2>/dev/null | sed -E 's/-l:lib([^ ]+)\.a/-l\1/g')"

    if PROBE_LOG=$(cc "$PROBE_DIR/probe.c" -o "$PROBE_DIR/probe" \
            "${PROBE_CFLAGS[@]}" "${PROBE_LIBS[@]}" 2>&1); then
        ok "trial link against libdpdk"
    else
        bad "trial link against libdpdk failed"
        # Collect every unresolved library the linker complained about.
        while read -r lib; do
            [[ -z "$lib" ]] && continue
            pkg=$(pkg_for_lib "$lib")
            if [[ -n "$pkg" ]]; then
                printf '      unresolved -l%s -> %s\n' "$lib" "$pkg"
                need "$pkg"
            else
                printf '      unresolved -l%s %s(no known package)%s\n' "$lib" "$C_WARN" "$C_OFF"
            fi
        done < <(printf '%s\n' "$PROBE_LOG" \
            | grep -oE "(unable to find library|cannot find) -l[^ ']+" \
            | sed -E -e 's/.*-l//' -e 's/[:,."]+$//' | sort -u)
        [[ "$VERBOSE" == true ]] && printf '%s%s%s\n' "$C_DIM" "$PROBE_LOG" "$C_OFF"
    fi
elif [[ "$DPDK_OK" == true ]]; then
    warn "skipping trial link (no C compiler)"
else
    warn "skipping trial link (libdpdk not installed)"
fi

head_ "Mellanox / Azure accelerated networking (optional)"
for spec in "libibverbs:libibverbs-dev" "librdmacm:librdmacm-dev"; do
    mod="${spec%%:*}"; pkg="${spec##*:}"
    if pkg-config --exists "$mod" 2>/dev/null; then
        ok "$mod"
    else
        warn "$mod (missing, needed for mlx5/MANA PMDs) -> $pkg"
        want "$pkg"
    fi
done

if [[ "$SOURCE_BUILD" == true ]]; then
    head_ "Source build of DPDK (optional path via CMakeLists.txt)"
    check_cmd cmake cmake optional
    check_cmd meson meson optional
    check_cmd ninja ninja-build optional "ninja"
    check_cmd dpkg-deb dpkg optional "dpkg-deb (for .deb packaging)"
fi

if [[ "$RUNTIME" == true ]]; then
    head_ "Runtime prerequisites"
    NR_HUGE=$(cat /sys/kernel/mm/hugepages/hugepages-2048kB/nr_hugepages 2>/dev/null || echo 0)
    if [[ "${NR_HUGE:-0}" -gt 0 ]]; then
        ok "hugepages allocated (nr_hugepages=$NR_HUGE)"
    else
        warn "no 2MB hugepages allocated"
        NOTES+=("Allocate hugepages: sudo sh -c 'echo 1024 > /sys/kernel/mm/hugepages/hugepages-2048kB/nr_hugepages'")
    fi

    if grep -qE 'hugetlbfs\s+/dev/hugepages' /proc/mounts 2>/dev/null; then
        ok "hugetlbfs mounted at /dev/hugepages"
    else
        warn "hugetlbfs not mounted at /dev/hugepages"
        NOTES+=("Mount hugetlbfs: sudo mkdir -p /dev/hugepages && sudo mount -t hugetlbfs none /dev/hugepages")
    fi

    if [[ "$(id -u)" -eq 0 ]]; then
        ok "running as root"
    else
        warn "not running as root (DPDK needs root for memory and device access)"
    fi
fi

# Deduplicate while preserving order.
dedupe() {
    local seen=" " item
    for item in "$@"; do
        [[ " $seen " == *" $item "* ]] && continue
        seen="$seen $item"
        printf '%s\n' "$item"
    done
}

mapfile -t MISSING_PKGS < <(dedupe "${MISSING_PKGS[@]+"${MISSING_PKGS[@]}"}")
mapfile -t OPTIONAL_PKGS < <(dedupe "${OPTIONAL_PKGS[@]+"${OPTIONAL_PKGS[@]}"}")

# Do not suggest an optional package that is already in the required list.
FILTERED_OPTIONAL=()
for pkg in ${OPTIONAL_PKGS[@]+"${OPTIONAL_PKGS[@]}"}; do
    [[ " ${MISSING_PKGS[*]-} " == *" $pkg "* ]] && continue
    FILTERED_OPTIONAL+=("$pkg")
done

head_ "Summary"
if [[ ${#MISSING_PKGS[@]} -eq 0 && $FAILED -eq 0 ]]; then
    printf '  %sAll required dependencies are present.%s\n' "$C_OK" "$C_OFF"
else
    printf '  %sMissing required dependencies.%s\n' "$C_BAD" "$C_OFF"
fi

INSTALL_CMD=""
if [[ ${#MISSING_PKGS[@]} -gt 0 ]]; then
    INSTALL_CMD="sudo apt-get install -y ${MISSING_PKGS[*]}"
    printf '\n  Install with:\n\n    %s\n' "$INSTALL_CMD"
fi

if [[ ${#FILTERED_OPTIONAL[@]} -gt 0 ]]; then
    printf '\n  Optional:\n\n    sudo apt-get install -y %s\n' "${FILTERED_OPTIONAL[*]}"
fi

if [[ ${#NOTES[@]} -gt 0 ]]; then
    printf '\n  Notes:\n'
    for note in "${NOTES[@]}"; do
        printf '    - %s\n' "$note"
    done
fi
echo

if [[ "$FIX" == true && -n "$INSTALL_CMD" ]]; then
    echo "Running: $INSTALL_CMD"
    # shellcheck disable=SC2086
    sudo apt-get install -y ${MISSING_PKGS[*]} || exit 1
    echo
    echo "Re-run ${BASH_SOURCE[0]} to verify."
fi

[[ $FAILED -eq 0 && ${#MISSING_PKGS[@]} -eq 0 ]]
