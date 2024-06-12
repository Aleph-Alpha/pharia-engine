#!/bin/bash

CONTAINER_NAME=$1

INTERNAL_NAME="test-container"

if [ -z $CONTAINER_NAME ]; then
    echo "missing container name."
    echo "usage: test-container.sh CONTAINER_NAME"
    exit 1
fi

if ! command -v podman &> /dev/null; then
    echo "Podman could not be found. Please install Podman."
    exit 1
fi

podman run -d -p 8081:8081 --name $INTERNAL_NAME $CONTAINER_NAME

SECONDS=0
START_TIME=$SECONDS
podman stop -f name=$INTERNAL_NAME
ELAPSED_TIME=$SECONDS

podman rm $INTERNAL_NAME

if [ "$ELAPSED_TIME" -gt 1 ]; then
    echo "shutdown time is longer than expected"
    exit 1
fi
