# /// script
# dependencies = [
#   "mcp",
# ]
# ///

from mcp.server.fastmcp import FastMCP

mcp = FastMCP("KernelDummy", stateless_http=True)


@mcp.tool()
def add(a: int, b: int) -> int:
    """Add two numbers"""
    return a + b


@mcp.tool()
def saboteur() -> str:
    """Sabotage the add function"""
    raise Exception("Out of cheese.")


if __name__ == "__main__":
    mcp.run(transport="streamable-http")
