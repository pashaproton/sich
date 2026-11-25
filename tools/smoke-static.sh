#!/usr/bin/env bash
# Black-box smoke test for static serving. No cargo test needed.
#   ./tools/smoke-static.sh target/debug/sich
set -uo pipefail

BIN=${1:-target/debug/sich}
[ -x "$BIN" ] || { echo "usage: $0 path/to/sich  (build first: cargo build)"; exit 2; }
BIN=$(readlink -f "$BIN")

WORK=$(mktemp -d)
cleanup() {
  if [ -n "${PID:-}" ]; then
    kill "$PID" 2>/dev/null
    wait "$PID" 2>/dev/null   # reap it, so bash stops shouting "Terminated"
  fi
  rm -rf "$WORK"
}
trap cleanup EXIT
mkdir -p "$WORK/www/sub" "$WORK/outside"
printf '<h1>hello</h1>' > "$WORK/www/index.html"
INDEX_LEN=$(wc -c < "$WORK/www/index.html" | tr -d '[:space:]')
printf 'secret\n' > "$WORK/outside/secret"
printf 'nested\n' > "$WORK/www/sub/page.html"
ln -s "$WORK/outside" "$WORK/www/link"
head -c 300000 /dev/urandom > "$WORK/www/big.bin"

PORT=$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1])')
cat > "$WORK/sich.toml" <<TOML
[[listeners]]
bind = "127.0.0.1:$PORT"
protocol = "http"

[[listeners.sites]]
hostname = "test.local"
root = "$WORK/www"
TOML

"$BIN" --config "$WORK/sich.toml" >"$WORK/server.log" 2>&1 &
PID=$!
for _ in $(seq 50); do curl -s -o /dev/null "http://127.0.0.1:$PORT/" && break; sleep 0.1; done

pass=0; fail=0
check() { # name expected_curl_format path
  local name=$1 want=$2 path=$3
  local got
  got=$(curl -s -o /dev/null -w '%{http_code} %{content_type} %{size_download}' \
        -H 'Host: test.local' "http://127.0.0.1:$PORT$path")
  if [ "$got" = "$want" ]; then
    echo "  ok    $name  ($got)"; pass=$((pass+1))
  else
    echo "  FAIL  $name  want [$want] got [$got]"; fail=$((fail+1))
  fi
}

# same as check, but forbids curl from normalising the path
check_as_is() { # name expected path
  local name=$1 want=$2 path=$3 got
  got=$(curl -s -o /dev/null --path-as-is -w '%{http_code} %{content_type} %{size_download}' \
        -H 'Host: test.local' "http://127.0.0.1:$PORT$path")
  if [ "$got" = "$want" ]; then
    echo "  ok    $name  ($got)"; pass=$((pass+1))
  else
    echo "  FAIL  $name  want [$want] got [$got]"; fail=$((fail+1))
  fi
}

echo "sich static smoke test on port $PORT"
check "index"              "200 text/html $INDEX_LEN"   "/index.html"
check "directory->index"   "200 text/html $INDEX_LEN"   "/"
check "nested"             "200 text/html 7"            "/sub/page.html"
check "missing"            "404 text/plain 13"          "/nope.txt"
check "no index in dir"    "404 text/plain 13"          "/sub"
# --path-as-is: without it curl collapses "/../" itself and we test curl, not sich
check_as_is "traversal"    "403 text/plain 13"          "/../etc/passwd"
check "encoded traversal"  "403 text/plain 13"          "/%2e%2e/etc/passwd"
check "symlink escape"     "403 text/plain 13"          "/link/secret"
check "big file"           "200 application/octet-stream 300000" "/big.bin"

body=$(curl -s -H 'Host: test.local' "http://127.0.0.1:$PORT/index.html")
[ "$body" = '<h1>hello</h1>' ] && { echo "  ok    body content"; pass=$((pass+1)); } \
  || { echo "  FAIL  body content [$body]"; fail=$((fail+1)); }

head_len=$(curl -s -I -H 'Host: test.local' "http://127.0.0.1:$PORT/index.html" | tr -d '\r' | awk -F': ' 'tolower($1)=="content-length"{print $2}')
[ "$head_len" = "$INDEX_LEN" ] && { echo "  ok    HEAD content-length"; pass=$((pass+1)); } \
  || { echo "  FAIL  HEAD content-length [$head_len]"; fail=$((fail+1)); }

code=$(curl -s -o /dev/null -w '%{http_code}' -X POST -H 'Host: test.local' "http://127.0.0.1:$PORT/index.html")
[ "$code" = "405" ] && { echo "  ok    POST -> 405"; pass=$((pass+1)); } \
  || { echo "  FAIL  POST -> $code"; fail=$((fail+1)); }

# big file integrity
curl -s -H 'Host: test.local' "http://127.0.0.1:$PORT/big.bin" -o "$WORK/got.bin"
if cmp -s "$WORK/got.bin" "$WORK/www/big.bin"; then echo "  ok    big file bytes"; pass=$((pass+1));
else echo "  FAIL  big file bytes"; fail=$((fail+1)); fi

echo "---"; echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ]
