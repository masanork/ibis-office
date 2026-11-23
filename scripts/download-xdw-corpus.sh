#!/bin/sh
# Download the optional public samples for ibis-xdw-read's ignored corpus test.
set -eu

if [ "$#" -ne 1 ] || [ -z "$1" ]; then
    printf 'Usage: %s CORPUS_DIR\n' "$0" >&2
    exit 2
fi

mkdir -p -- "$1"
destination=$(cd -- "$1" && pwd)
scratch=$(mktemp -d "$destination/.xdw-download.XXXXXX")
trap 'rm -rf -- "$scratch"' EXIT
trap 'exit 1' HUP INT TERM

download() {
    name=$1
    url=$2
    printf 'Downloading %s\n' "$name"
    curl --fail --silent --show-error --location --retry 2 \
        --proto '=https' --proto-redir '=https' --max-time 60 \
        --output "$scratch/$name" "$url"
    # DocuWorks files begin with 60 0e; reject HTML error pages returned as HTTP 200.
    if [ "$(LC_ALL=C od -An -N2 -tx1 "$scratch/$name" | tr -d '[:space:]')" != 600e ]; then
        printf 'Not a DocuWorks file: %s\n' "$url" >&2
        exit 1
    fi
}

download codelibs-test_test_xdw_ver8.xdw https://raw.githubusercontent.com/codelibs/fess-testdata/master/files/docuworks/test_xdw_ver8.xdw
download codelibs-test_test_xdw_ver9.xdw https://raw.githubusercontent.com/codelibs/fess-testdata/master/files/docuworks/test_xdw_ver9.xdw
download koto-ku_r604kinyumihon.xdw https://www.city.koto.lg.jp/212103/fukushi/kaigohoken/shiryo/documents/r604kinyumihon.xdw
download ohta-geo_rocfall.xdw https://ohta-geo.co.jp/products/pamphlet/rocfall.xdw
download okinawa-pref_63.xdw https://www.pref.okinawa.jp/_res/projects/default_project/_page_/001/016/409/63.xdw
download okinawa-pref_79.xdw https://www.pref.okinawa.jp/_res/projects/default_project/_page_/001/016/409/79.xdw
download saitama-city_taishouichiran_kyouryou.xdw https://www.city.saitama.lg.jp/001/010/018/007/002/p010863_d/fil/taishouichiran_kyouryou.xdw

mv -- "$scratch"/*.xdw "$destination/"
printf 'Downloaded the XDW corpus to %s\n' "$destination"
