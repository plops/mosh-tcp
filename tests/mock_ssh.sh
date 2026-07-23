#!/bin/bash
# mock_ssh.sh - Helper script for mosh-tcp SSH login integration tests

LOCAL_PORT=""
REMOTE_CMD=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        -L)
            FWD="$2"
            LOCAL_PORT="${FWD%%:*}"
            shift 2
            ;;
        -o|-p)
            shift 2
            ;;
        *)
            if [[ "$1" == *"mosh-tcp-server"* ]]; then
                REMOTE_CMD="$1"
            fi
            shift
            ;;
    esac
done

if [ -n "$LOCAL_PORT" ] && [ -n "$REMOTE_CMD" ]; then
    CMD=$(echo "$REMOTE_CMD" | sed -E "s/--bind 127\.0\.0\.1:[0-9]+/--bind 127.0.0.1:$LOCAL_PORT/")
    exec $CMD
fi
