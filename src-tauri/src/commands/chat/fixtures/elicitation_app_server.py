import json
import os
import signal
import sys


def emit(message):
    print(json.dumps(message), flush=True)


def notify(method, params):
    emit({"method": method, "params": {"threadId": "thread", **params}})


for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    request_id = message.get("id")
    if method == "initialize":
        emit({"id": request_id, "result": {}})
    elif method == "account/read":
        emit({"id": request_id, "result": {"account": {}}})
    elif method == "thread/start":
        emit({"id": request_id, "result": {"thread": {"id": "thread", "turns": []}}})
    elif method == "turn/start":
        emit({"id": request_id, "result": {"turn": {"id": "turn"}}})
        notify("turn/started", {"turn": {"id": "turn"}})
        for pending_id in [90, 91]:
            emit({"id": pending_id, "method": "mcpServer/elicitation/request", "params": {
                "threadId": "thread", "turnId": "turn", "serverName": "test-mcp",
                "mode": "form", "message": "Confirm download", "requestedSchema": {
                    "type": "object", "required": ["download"],
                    "properties": {"download": {"type": "boolean"}}
                }
            }})
        notify("item/started", {"item": {"id": "progress", "type": "agentMessage", "text": ""}})
        notify("item/agentMessage/delta", {"itemId": "progress", "delta": "Still streaming"})
        notify("serverRequest/resolved", {"requestId": 91})
        notify("thread/tokenUsage/updated", {"tokenUsage": {"last": {"totalTokens": 123}, "modelContextWindow": 1000}})
    elif method == "turn/interrupt":
        emit({"id": request_id, "result": {}})
        notify("turn/completed", {"turn": {"id": "turn", "status": "interrupted"}})
    elif method is None and "result" in message:
        result = message["result"]
        notify("item/completed", {"item": {
            "id": "reply-" + str(request_id), "type": "agentMessage",
            "text": json.dumps({"id": request_id, "result": result})
        }})
        notify("serverRequest/resolved", {"requestId": request_id})
    elif method == "thread/compact/start":
        sys.exit(0)
    elif method == "test/closeInput":
        os.close(0)
        notify("item/completed", {"item": {"id": "stdin-closed", "type": "agentMessage", "text": "Input closed"}})
        signal.pause()
    elif request_id is not None:
        emit({"id": request_id, "result": {}})
