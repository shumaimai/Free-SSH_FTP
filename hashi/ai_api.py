"""APIキー方式のResponses/Anthropic/互換Chat Completions。"""
from __future__ import annotations

import json

from .ai_http import JsonHttp, normalize_api_base

COMMANDCODE_BASE = "https://api.commandcode.ai/provider/v1"


def responses_input(messages):
    items = []
    for m in messages:
        if m["role"] == "tool":
            items.append({"type": "function_call_output", "call_id": m["tool_call_id"], "output": m["content"]})
        elif "raw_output" in m:
            items.extend(m["raw_output"])
        else:
            if m.get("content"):
                items.append({"role": m["role"], "content": m["content"]})
            items.extend({"type": "function_call", "call_id": c["id"], "name": c["name"],
                          "arguments": json.dumps(c["arguments"])} for c in m.get("tool_calls", []))
    return items


class ApiProvider:
    def __init__(self, kind, model, key, *, base_url=None, http=None, use_tools=True,
                 protocol="auto"):
        defaults = {"openai": "https://api.openai.com/v1", "anthropic": "https://api.anthropic.com/v1"}
        if kind not in {*defaults, "compatible", "commandcode"} or not model.strip():
            raise ValueError("プロバイダーとモデルを指定してください")
        self.kind, self.model, self._key = kind, model.strip(), key
        self.base = normalize_api_base(base_url or (COMMANDCODE_BASE if kind == "commandcode"
                                                    else defaults.get(kind, "")))
        if kind in defaults and self.base != defaults[kind]:
            raise ValueError("公式API方式は公式接続先を使います。独自URLは互換方式を選択してください")
        self.http = http or JsonHttp()
        self.use_tools = bool(use_tools)
        if protocol not in {"auto", "chat", "responses", "messages"}:
            raise ValueError("APIの形式を選んでください")
        self.protocol = protocol
        if protocol == "auto":
            self.protocol = ("responses" if kind == "openai" else "messages"
                             if kind == "anthropic" or (self.base == COMMANDCODE_BASE
                                                        and "claude" in self.model.lower()) else "chat")

    def models(self, cancel):
        if cancel.is_set():
            raise InterruptedError("接続確認を停止しました")
        headers = self._headers()
        data = self.http.json(self.base + "/models", headers=headers)
        if cancel.is_set():
            raise InterruptedError("接続確認を停止しました")
        return [m for m in data.get("data", []) if isinstance(m, dict) and isinstance(m.get("id"), str)]

    def _headers(self):
        if self.kind == "anthropic":
            return {"x-api-key": self._key, "anthropic-version": "2023-06-01"}
        return {"Authorization": "Bearer " + self._key} if self._key else {}

    def respond(self, messages, instructions, tools, cancel, on_text):
        enabled = tools if self.use_tools else []
        if self.protocol == "responses":
            reply = self._responses(messages, instructions, enabled, cancel, on_text)
        elif self.protocol == "messages":
            reply = self._anthropic(messages, instructions, enabled, cancel, on_text)
        else:
            reply = self._chat(messages, instructions, enabled, cancel, on_text)
        if not self.use_tools and reply.get("tool_calls"):
            raise RuntimeError("相談専用の接続がツールを要求しました。実行していません")
        return reply

    def _responses_payload(self, messages, instructions, tools):
        payload = {"model": self.model, "store": False, "stream": True,
                "include": ["reasoning.encrypted_content"], "instructions": instructions,
                "input": responses_input(messages),
                "tools": [{"type": "function", "strict": True, **t} for t in tools]}
        if not self.use_tools:
            payload.pop("tools")
        return payload

    def _responses(self, messages, instructions, tools, cancel, on_text):
        completed, text = None, []
        for e in self.http.events(self.base + "/responses", self._responses_payload(messages, instructions, tools),
                                  self._headers(), cancel):
            kind = e.get("type")
            if kind == "response.output_text.delta":
                text.append(e["delta"])
                on_text(e["delta"])
            elif kind == "response.completed":
                completed = e["response"]
            elif kind in {"error", "response.failed", "response.incomplete"}:
                raise RuntimeError("AI応答が完了しませんでした。利用上限と接続設定を確認してください")
        if completed is None:
            raise RuntimeError("AI応答が途中で切れました。ツールは実行していません")
        output = completed.get("output", [])
        calls = [{"id": i["call_id"], "name": i["name"], "arguments": json.loads(i["arguments"])}
                 for i in output if i.get("type") == "function_call"]
        final = "".join(text) or "".join(b.get("text", "") for i in output if i.get("type") == "message"
                                       for b in i.get("content", []) if b.get("type") == "output_text")
        return {"text": final, "tool_calls": calls, "raw_output": output, "usage": completed.get("usage", {})}

    def _anthropic(self, messages, instructions, tools, cancel, on_text):
        history = []
        for m in messages:
            role = m["role"]
            if role == "tool":
                role, blocks = "user", [{"type": "tool_result", "tool_use_id": m["tool_call_id"], "content": m["content"]}]
            else:
                blocks = ([{"type": "text", "text": m["content"]}] if m.get("content") else [])
                blocks += [{"type": "tool_use", "id": c["id"], "name": c["name"], "input": c["arguments"]}
                           for c in m.get("tool_calls", [])]
            if history and history[-1]["role"] == role:
                history[-1]["content"].extend(blocks)
            elif blocks:
                history.append({"role": role, "content": blocks})
        payload = {"model": self.model, "system": instructions, "max_tokens": 4096, "stream": True,
                   "messages": history, "tools": [{"name": t["name"], "description": t["description"],
                   "input_schema": t["parameters"]} for t in tools]}
        if not self.use_tools:
            payload.pop("tools")
        text, calls, ended, reason = [], {}, False, None
        for e in self.http.events(self.base + "/messages", payload,
                {**self._headers(), "anthropic-version": "2023-06-01"}, cancel):
            kind = e.get("type")
            if kind == "error":
                raise RuntimeError("Anthropicの応答エラー。利用上限を確認してください")
            if kind == "content_block_start" and e["content_block"]["type"] == "tool_use":
                block = e["content_block"]
                calls[e["index"]] = {"id": block["id"], "name": block["name"], "json": ""}
            elif kind == "content_block_delta":
                delta = e["delta"]
                if delta["type"] == "text_delta":
                    text.append(delta["text"])
                    on_text(delta["text"])
                elif delta["type"] == "input_json_delta":
                    calls[e["index"]]["json"] += delta["partial_json"]
            elif kind == "message_delta":
                reason = e.get("delta", {}).get("stop_reason")
            elif kind == "message_stop":
                ended = True
        if not ended or reason not in {"end_turn", "tool_use", "stop_sequence"}:
            raise RuntimeError("Anthropic応答が未完了です。ツールは実行していません")
        return {"text": "".join(text), "tool_calls": [{"id": c["id"], "name": c["name"],
                 "arguments": json.loads(c["json"] or "{}")} for c in calls.values()]}

    def _chat(self, messages, instructions, tools, cancel, on_text):
        history = [{"role": "system", "content": instructions}]
        for m in messages:
            item = {"role": m["role"], "content": m["content"]}
            if m["role"] == "tool":
                item["tool_call_id"] = m["tool_call_id"]
            if m.get("tool_calls"):
                item["tool_calls"] = [{"id": c["id"], "type": "function", "function":
                    {"name": c["name"], "arguments": json.dumps(c["arguments"])}} for c in m["tool_calls"]]
            history.append(item)
        payload = {"model": self.model, "messages": history, "stream": True}
        if self.use_tools:
            payload["tools"] = [{"type": "function", "function": t} for t in tools]
        text, calls, ended = [], {}, False
        for e in self.http.events(self.base + "/chat/completions", payload,
                                  self._headers(), cancel):
            if e.get("error"):
                raise RuntimeError("互換APIの応答エラー")
            for choice in e.get("choices", []):
                if choice.get("finish_reason") in {"stop", "tool_calls"}:
                    ended = True
                delta = choice.get("delta", {})
                if delta.get("content"):
                    text.append(delta["content"])
                    on_text(delta["content"])
                for call in delta.get("tool_calls", []):
                    if not self.use_tools:
                        raise RuntimeError("相談専用の互換APIがツールを要求しました。実行していません")
                    c = calls.setdefault(call["index"], {"id": "", "name": "", "json": ""})
                    c["id"] += call.get("id", "")
                    c["name"] += call.get("function", {}).get("name", "")
                    c["json"] += call.get("function", {}).get("arguments", "")
        if not ended:
            raise RuntimeError("互換API応答が未完了です。ツールは実行していません")
        return {"text": "".join(text), "tool_calls": [{"id": c["id"], "name": c["name"],
                 "arguments": json.loads(c["json"] or "{}")} for c in calls.values()]}
