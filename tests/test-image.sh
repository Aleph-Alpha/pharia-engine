#!/bin/bash

PORT=$1
HOST=${2-localhost}

if [ -z $PORT ]; then
    echo "missing published port."
    echo "usage: test-image.sh PUBLISHED_PORT [HOST_NAME]"
    exit 1
fi

BODY=$(curl -s http://$HOST:$PORT)

if [ "$BODY" = "Hello, world!" ]; then
    exit 0
else
    echo "unexpected response: BODY='$BODY'"
    exit 1
fi
