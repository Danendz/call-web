#!/usr/bin/env python3
"""Drives a running dev build through Slint's embedded MCP server: finds elements by accessible label (what `@tr`
shows in the current language), presses them, sets text, and screenshots the window. Unlike ui.swift it works with
the screen locked and on Windows, but it needs a build made for it:

    SLINT_EMIT_DEBUG_INFO=1 cargo build -p call-danya --features dev-mcp
    SLINT_MCP_PORT=8765 CALL_SERVER=http://localhost:8090 ./target/debug/call-danya

    python3 dev/ui.py tree                   accessible elements: role, label, value
    python3 dev/ui.py press <label>          press a button
    python3 dev/ui.py set <label> <text>     set a text field's text
    python3 dev/ui.py shot <file.png>        screenshot the window

UI_PORT picks another port (default 8765).
"""

import base64
import json
import os
import sys
import urllib.request

URL = f"http://127.0.0.1:{os.environ.get('UI_PORT', '8765')}/mcp"


def call(tool, **arguments):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": tool, "arguments": arguments}})
    request = urllib.request.Request(
        URL, body.encode(), {"Content-Type": "application/json", "Accept": "application/json, text/event-stream"}
    )
    try:
        with urllib.request.urlopen(request, timeout=10) as response:
            reply = json.load(response)
    except OSError as e:
        sys.exit(f"no app on {URL} ({e}); see the top of this file for how to start one")
    if "error" in reply:
        sys.exit(f"{tool}: {reply['error']}")
    content = reply["result"]["content"][0]
    if reply["result"].get("isError"):
        sys.exit(f"{tool}: {content.get('text')}")
    return content


def window():
    windows = json.loads(call("list_windows")["text"])["windowHandles"]
    if not windows:
        sys.exit("the app has no window")
    return windows[0]


def elements():
    root = json.loads(call("get_window_properties", windowHandle=window())["text"])["rootElementHandle"]
    tree = json.loads(call("get_element_tree", elementHandle=root, maxElements=1000)["text"])
    if tree.get("debugInfoMissing"):
        sys.exit("built without SLINT_EMIT_DEBUG_INFO=1: the tree is empty")
    # Only what has an accessible role, as a screen reader sees it
    return [e for e in tree["elements"] if e.get("accessibleRole")]


def find(label):
    matches = [e for e in elements() if label in (e.get("accessibleLabel"), e.get("accessibleValue"))]
    if not matches:
        sys.exit(f'nothing labelled "{label}"; see `tree`')
    # A button's label is also on the text inside it: the one that acts is the one with an action
    return next((e for e in matches if e["accessibleRole"] != "Text"), matches[0])


def main(args):
    match args:
        case ["tree"]:
            for e in elements():
                parts = [e["accessibleRole"], e.get("accessibleLabel") or "", e.get("accessibleValue") or ""]
                print("  ".join(p for p in parts if p))
        case ["press", label]:
            call("invoke_accessibility_action", elementHandle=find(label)["handle"], action="Default_")
        case ["set", label, text]:
            call("set_element_value", elementHandle=find(label)["handle"], value=text)
        case ["shot", path]:
            image = call("take_screenshot", windowHandle=window())
            with open(path, "wb") as f:
                f.write(base64.b64decode(image["data"]))
        case _:
            sys.exit(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])
