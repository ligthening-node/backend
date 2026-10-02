#!/usr/bin/env bash
# Regtest helpers for the docker compose stack.
#
#   scripts/regtest.sh mine [blocks]           mine blocks (default 6)
#   scripts/regtest.sh fund <address> [btc]    send coins from bitcoind's wallet (default 0.1) and mine 6
#   scripts/regtest.sh peer <path> [json]      call the peer node's API, e.g. peer node/status
#                                              or peer invoices '{"amount_msat":"50000000","description":"coffee"}'
#
# Needs LN_API_TOKEN (the one given to docker compose) for peer calls.
set -euo pipefail

RPC_URL="${RPC_URL:-http://127.0.0.1:18443}"
RPC_AUTH="${RPC_AUTH:-lightning:lightning}"
PEER_URL="${PEER_URL:-http://127.0.0.1:3002}"
WALLET="regtest"

rpc() {
  local path="$1" method="$2" params="$3"
  local response
  response=$(curl -sS --user "$RPC_AUTH" -H 'content-type: text/plain' \
    --data "{\"jsonrpc\":\"1.0\",\"id\":\"regtest.sh\",\"method\":\"$method\",\"params\":$params}" \
    "$RPC_URL$path")
  if [[ "$response" != *'"error":null'* ]]; then
    echo "$response" >&2
    return 1
  fi
  echo "$response" | sed -E 's/^\{"result":(.*),"error":null,"id":"regtest.sh"\}$/\1/'
}

ensure_wallet() {
  # Creating fails when the wallet exists, loading fails when it is loaded: either is fine.
  rpc "" createwallet "[\"$WALLET\"]" >/dev/null 2>&1 || true
  rpc "" loadwallet "[\"$WALLET\"]" >/dev/null 2>&1 || true
  local balance
  balance=$(rpc "/wallet/$WALLET" getbalance "[]")
  # Coinbase outputs need 100 confirmations before they can be spent.
  if awk "BEGIN { exit !($balance < 1) }"; then
    mine 101 >/dev/null
  fi
}

mine() {
  local blocks="${1:-6}" address
  rpc "" createwallet "[\"$WALLET\"]" >/dev/null 2>&1 || true
  rpc "" loadwallet "[\"$WALLET\"]" >/dev/null 2>&1 || true
  address=$(rpc "/wallet/$WALLET" getnewaddress "[]" | tr -d '"')
  rpc "/wallet/$WALLET" generatetoaddress "[$blocks, \"$address\"]" >/dev/null
  echo "mined $blocks block(s)"
}

fund() {
  local address="$1" btc="${2:-0.1}" txid
  ensure_wallet
  txid=$(rpc "/wallet/$WALLET" sendtoaddress "[\"$address\", $btc]" | tr -d '"')
  mine 6 >/dev/null
  echo "sent $btc BTC to $address in $txid (confirmed)"
}

peer() {
  local path="$1" body="${2:-}"
  : "${LN_API_TOKEN:?set LN_API_TOKEN to the token given to docker compose}"
  if [[ -z "$body" ]]; then
    curl -sS -H "authorization: Bearer $LN_API_TOKEN" "$PEER_URL/$path"
  else
    curl -sS -H "authorization: Bearer $LN_API_TOKEN" -H 'content-type: application/json' \
      --data "$body" "$PEER_URL/$path"
  fi
  echo
}

case "${1:-}" in
  mine) mine "${2:-6}" ;;
  fund) fund "${2:?address required}" "${3:-0.1}" ;;
  peer) peer "${2:?path required}" "${3:-}" ;;
  *)
    sed -n '2,9p' "$0" | sed 's/^# \{0,1\}//'
    exit 1
    ;;
esac
