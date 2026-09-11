#!/bin/sh
# fake_mcp_server.sh - a minimal MCP stdio server for ah-plugins-mcp tests.
#
# Speaks newline-delimited JSON-RPC 2.0 over stdin/stdout (the real MCP stdio
# transport shape): one JSON object per line. Answers requests (messages that
# carry an "id") with a JSON-RPC response on stdout; ignores notifications
# (messages without "id"); exits when stdin reaches EOF.
#
# Requests handled: initialize, tools/list, tools/call (echo / add), shutdown.
# Unknown tools are answered with a JSON-RPC error (code -32602).
#
# If MARKER_DIR (or $1) is set, creates "$MARKER_DIR/exited" just before
# exiting so tests can assert the server process really terminated.

marker_dir="${1:-${MARKER_DIR:-}}"

json_field() {
    # $1 = field name, $2 = JSON line; prints the first string value found.
    echo "$2" | sed -n 's/.*"'"$1"'"[[:space:]]*:[[:space:]]*"\([^"\\]*\)".*/\1/p'
}

json_int() {
    # $1 = field name, $2 = JSON line; prints the first integer value found.
    echo "$2" | sed -n 's/.*"'"$1"'"[[:space:]]*:[[:space:]]*\([0-9][0-9]*\).*/\1/p'
}

while IFS= read -r line; do
    [ -z "$line" ] && continue
    case "$line" in
        *'"id"'*) ;;
        *) continue ;;   # notification (no id): ignore
    esac

    id=$(json_int id "$line")
    [ -z "$id" ] && continue
    method=$(json_field method "$line")

    case "$method" in
        initialize)
            printf '%s\n' '{"jsonrpc":"2.0","id":'"$id"',"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"fake-mcp-server","version":"0.1.0"}}}'
            ;;
        tools/list)
            printf '%s\n' '{"jsonrpc":"2.0","id":'"$id"',"result":{"tools":[{"name":"echo","description":"echo back a text argument","inputSchema":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"]}},{"name":"add","description":"add two integers","inputSchema":{"type":"object","properties":{"a":{"type":"integer"},"b":{"type":"integer"}},"required":["a","b"]}},{"name":"render_image","description":"return an image content block","inputSchema":{"type":"object"}}]}}'
            ;;
        tools/call)
            name=$(json_field name "$line")
            case "$name" in
                echo)
                    text=$(json_field text "$line")
                    printf '%s\n' '{"jsonrpc":"2.0","id":'"$id"',"result":{"content":[{"type":"text","text":"echo:'"$text"'"}],"isError":false}}'
                    ;;
                add)
                    a=$(json_int a "$line")
                    b=$(json_int b "$line")
                    sum=$((a + b))
                    printf '%s\n' '{"jsonrpc":"2.0","id":'"$id"',"result":{"content":[{"type":"text","text":"'"$sum"'"}],"isError":false}}'
                    ;;
                render_image)
                    printf '%s\n' '{"jsonrpc":"2.0","id":'"$id"',"result":{"content":[{"type":"image","data":"aGVsbG8gcG5n","mimeType":"image/png"},{"type":"text","text":"rendered"}],"isError":false}}'
                    ;;
                *)
                    printf '%s\n' '{"jsonrpc":"2.0","id":'"$id"',"error":{"code":-32602,"message":"Unknown tool: '"$name"'"}}'
                    ;;
            esac
            ;;
        shutdown)
            printf '%s\n' '{"jsonrpc":"2.0","id":'"$id"',"result":null}'
            ;;
        *)
            printf '%s\n' '{"jsonrpc":"2.0","id":'"$id"',"error":{"code":-32601,"message":"Method not found: '"$method"'"}}'
            ;;
    esac
done

if [ -n "$marker_dir" ]; then
    touch "$marker_dir/exited" 2>/dev/null || true
fi
exit 0
