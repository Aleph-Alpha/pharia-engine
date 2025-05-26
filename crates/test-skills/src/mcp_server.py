# /// script
# dependencies = [
#   "mcp",
# ]
# ///

import argparse

from mcp.server.fastmcp import FastMCP


def main():
    # MCP servers may respond with either text/event-stream or application/json.
    # For our tests, we want to go against both.
    parser = argparse.ArgumentParser(
        description="MCP Server with configurable JSON response"
    )
    parser.add_argument(
        "--json-response",
        action="store_true",
        help="Return responses in application/json format (default: False)",
    )

    parser.add_argument(
        "--port",
        type=int,
        default=8000,
        help="Port to listen on (default: 8000)",
    )

    args = parser.parse_args()
    print(
        f"Starting MCP server on port {args.port} with JSON response: {args.json_response}"
    )

    mcp = FastMCP(
        "KernelDummy",
        stateless_http=True,
        json_response=args.json_response,
        port=args.port,
    )

    @mcp.tool()
    def add(a: int, b: int) -> int:
        """Add two numbers"""
        return a + b

    @mcp.tool()
    def saboteur() -> str:
        """Sabotage the add function"""
        raise Exception("Out of cheese.")

    mcp.run(transport="streamable-http")


if __name__ == "__main__":
    main()
