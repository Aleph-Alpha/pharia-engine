#!/bin/bash

PORT=$1
HOST=${2-localhost}

if [ -z $PORT ]; then
    echo "missing published port."
    echo "usage: test-image.sh PUBLISHED_PORT [HOST_NAME]"
    exit 1
fi

for i in {1..5}
do
    BODY=$(curl -s http://$HOST:$PORT/health)
    if [ "$BODY" = "ok" ]; then
        break
    else
        sleep 1
    fi
done

if [ "$BODY" != "ok" ]; then
    echo "unexpected response: BODY='$BODY'"
    exit 1
fi

RESPONSE_CODE=$(curl -o /dev/null -s -w "%{http_code}\n" http://$HOST:$PORT/docs/index.html)

if [ "$RESPONSE_CODE" = "200" ]; then
    exit 0
else
    echo "unexpected response code: RESPONSE_CODE='$RESPONSE_CODE'"
    exit 1
fi
