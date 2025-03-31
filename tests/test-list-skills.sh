#!/bin/bash

TOKEN=$1
HOST=${2-http://127.0.0.1:8081}

echo "Listing skills..."
response=$(curl -sS $HOST/v1/skills -H "Authorization: Bearer $TOKEN")

if [ -z "$response" ]; then
    echo "Error: No data returned"
    exit 1
fi

if ! echo $response | grep -q "playground/haiku"; then
    echo "Error: Array does not contain 'playground/haiku'"
    exit 1
fi
