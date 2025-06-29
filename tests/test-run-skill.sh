#!/bin/bash

TOKEN=$1
HOST=${2-http://127.0.0.1:8081}

echo "Executing skill..."
RESPONSE_CODE=$(curl -w '%{http_code}' -s -o output.result \
                $HOST/v1/skills/playground/haiku/run \
                -H "Authorization: Bearer $TOKEN" \
                -H 'Content-Type: application/json' \
                -d '{"topic": "Oat milk"}' )

echo "Skill response:"
cat output.result
echo ""

if [ "$RESPONSE_CODE" = "200" ]; then
    exit 0
else
    echo "unexpected response code: RESPONSE_CODE='$RESPONSE_CODE'"
    exit 1
fi
