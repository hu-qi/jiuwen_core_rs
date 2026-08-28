#!/usr/bin/env python3
# coding: utf-8
"""Python reference runner for agent-harness P0-01 (Python/Rust differential).

Drives the REAL openjiuwen (agent-core) Python implementations for the seams
under test, using the same language-neutral fixtures as the Rust side, and
emits normalized outcomes to ``out/python/{seam}.json``.

Isolation loading
-----------------
openjiuwen's package ``__init__.py`` files aggregate-import unrelated heavy
subsystems (aiohttp/jsonschema/etc.). For a *module-scoped* differential we
load only the real source files under test via ``importlib``, pre-seeding
parent packages as empty packages so their ``__init__`` aggregation imports
are not executed. The module file itself runs verbatim, so the observable
behavior compared is the real implementation's. Logging is excluded from the
comparison surface (both sides log differently anyway), so
``openjiuwen.core.common.logging`` is shimmed with a null logger.

Discipline (docs/testing.md):
- This runner NEVER writes to ``references/`` (the Rust baseline). It only
  produces an independent Python outcome for comparison.
- Python >= 3.11 required (openjiuwen requires >=3.11,<3.14).
- External deps for current seams: pydantic only.

Usage:
    AGENT_CORE_ROOT=/path/to/agent-core python differential/run_python.py
"""

from __future__ import annotations

import asyncio
import importlib.util
import json
import logging
import os
import sys
import types
from pathlib import Path

HERE = Path(__file__).resolve().parent
FIXTURES = HERE.parent / "fixtures"  # 语言中立 fixture 统一在仓库根 fixtures/
OUT = HERE / "out" / "python"

AGENT_CORE_ROOT = Path(
    os.environ.get("AGENT_CORE_ROOT", "/Volumes/coder/开源/rs_jiuwen/agent-core")
)

_LOADED: set[str] = set()


def load_module_file(name: str) -> types.ModuleType:
    """Load a real agent-core module file under its dotted name.

    Parent packages are pre-seeded as empty packages so their ``__init__``
    aggregation imports are skipped; only the named module file executes.
    """
    if name in _LOADED:
        return sys.modules[name]
    parts = name.split(".")
    for i in range(1, len(parts)):
        pkg = ".".join(parts[:i])
        if pkg not in sys.modules:
            mod = types.ModuleType(pkg)
            mod.__path__ = []  # mark as package
            sys.modules[pkg] = mod
    path = AGENT_CORE_ROOT.joinpath(*parts).with_suffix(".py")
    if not path.exists():
        raise FileNotFoundError(f"{name} not found at {path}")
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    _LOADED.add(name)
    spec.loader.exec_module(mod)
    return mod


def shim_logging() -> None:
    """Null logger for openjiuwen.core.common.logging.

    The real logging chain pulls in core/common (BaseCard/StatusCode etc.).
    Log lines are not part of the differential comparison surface, so a null
    logger keeps the runner dependency-light without affecting behavior.
    """
    pkg = "openjiuwen.core.common"
    for i in range(1, len(pkg.split(".")) + 1):
        prefix = ".".join(pkg.split(".")[:i])
        if prefix not in sys.modules:
            m = types.ModuleType(prefix)
            m.__path__ = []
            sys.modules[prefix] = m
    leaf = types.ModuleType("openjiuwen.core.common.logging")
    logger = logging.getLogger("openjiuwen.team.differential")
    logger.addHandler(logging.NullHandler())
    leaf.team_logger = logger
    sys.modules["openjiuwen.core.common.logging"] = leaf
    _LOADED.add("openjiuwen.core.common.logging")


def load_fixture(name: str) -> dict:
    with (FIXTURES / f"{name}.json").open(encoding="utf-8") as fh:
        return json.load(fh)


def write_outcome(name: str, outcome: dict) -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / f"{name}.json").write_text(
        json.dumps(outcome, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    print(f"[python] wrote {OUT / (name + '.json')}")


# ---------------------------------------------------------------------------
# seam: stop_condition
# ---------------------------------------------------------------------------

def run_stop_condition() -> dict:
    sc = load_module_file("openjiuwen.harness.schema.stop_condition")
    fixture = load_fixture("stop_condition")
    cases_out = []
    for case in fixture["cases"]:
        ev = case["evaluator"]
        etype = ev["type"]
        if etype == "max_rounds":
            evaluator = sc.MaxRoundsEvaluator(max_rounds=ev["max_rounds"])
            decisions = [
                evaluator.should_stop(sc.StopEvaluationContext(**ctx))
                for ctx in case["contexts"]
            ]
            cases_out.append({"name": case["name"], "decisions": decisions})
        elif etype == "token_budget":
            evaluator = sc.TokenBudgetEvaluator(max_tokens=ev["max_tokens"])
            decisions = [
                evaluator.should_stop(sc.StopEvaluationContext(**ctx))
                for ctx in case["contexts"]
            ]
            cases_out.append({"name": case["name"], "decisions": decisions})
        elif etype == "timeout":
            evaluator = sc.TimeoutEvaluator(timeout_seconds=ev["timeout_seconds"])
            decisions = [
                evaluator.should_stop(sc.StopEvaluationContext(**ctx))
                for ctx in case["contexts"]
            ]
            cases_out.append({"name": case["name"], "decisions": decisions})
        elif etype == "completion_promise":
            evaluator = sc.CompletionPromiseEvaluator(
                promise=ev["promise"],
                required_confirmations=ev["required_confirmations"],
            )
            results = []
            for step in case["steps"]:
                op = step["op"]
                if op == "notify_fulfilled":
                    evaluator.notify_fulfilled(step["text"])
                elif op == "notify_absent":
                    evaluator.notify_absent()
                elif op == "should_stop":
                    results.append(evaluator.should_stop(sc.StopEvaluationContext()))
                elif op == "get_state":
                    results.append(evaluator.get_state())
                elif op == "load_state":
                    evaluator.load_state(step["state"])
                elif op == "reset":
                    evaluator.reset()
                else:
                    raise ValueError(f"unknown op {op}")
            cases_out.append({"name": case["name"], "results": results})
        else:
            raise ValueError(f"unknown evaluator type {etype}")
    return {"seam": "stop_condition", "cases": cases_out}


# ---------------------------------------------------------------------------
# seam: messager_inprocess
# ---------------------------------------------------------------------------

def _make_message_model():
    """Minimal pydantic message mirroring EventMessage's sender_id stamping
    contract, so the differential drives the REAL stamping/publish path of
    the loaded inprocess.py source."""
    from pydantic import BaseModel

    class DiffMessage(BaseModel):
        kind: str
        sender_id: str = ""

    return DiffMessage


async def run_messager_inprocess_async() -> dict:
    # Load the real source files in dependency order.
    load_module_file("openjiuwen.agent_teams.workflow.engine.progress")
    load_module_file("openjiuwen.agent_teams.schema.events")
    load_module_file("openjiuwen.agent_teams.messager.messager")
    base = load_module_file("openjiuwen.agent_teams.messager.base")
    inprocess = load_module_file("openjiuwen.agent_teams.messager.inprocess")

    DiffMessage = _make_message_model()
    fixture = load_fixture("messager_inprocess")
    cases_out = []
    for case in fixture["cases"]:
        inprocess.cleanup_inprocess_bus()
        deliveries: list[dict] = []

        async def handler(msg) -> None:
            deliveries.append(msg.model_dump())

        leader = base.create_messager(
            base.MessagerTransportConfig(node_id=case["config"]["node_id"])
        )
        peer = None
        if case.get("peer_config"):
            peer = base.create_messager(
                base.MessagerTransportConfig(node_id=case["peer_config"]["node_id"])
            )
        await leader.start()
        if peer is not None:
            await peer.start()

        for step in case["steps"]:
            op = step["op"]
            m = leader if step.get("who", "leader") == "leader" else peer
            if op == "subscribe":
                await m.subscribe(step["topic"], handler)
            elif op == "unsubscribe":
                await m.unsubscribe(step["topic"])
            elif op == "publish":
                await m.publish(step["topic"], DiffMessage(**step["message"]))
            elif op == "send":
                await m.send(step["agent_id"], DiffMessage(**step["message"]))
            elif op == "register_direct_message_handler":
                await m.register_direct_message_handler(handler)
            elif op == "unregister_direct_message_handler":
                await m.unregister_direct_message_handler()
            else:
                raise ValueError(f"unknown op {op}")

        await leader.stop()
        if peer is not None:
            await peer.stop()
        inprocess.cleanup_inprocess_bus()
        entry = {"name": case["name"], "deliveries": deliveries}
        if case.get("known_divergence"):
            entry["known_divergence"] = True
        cases_out.append(entry)
    return {"seam": "messager_inprocess", "cases": cases_out}


def run_messager_inprocess() -> dict:
    return asyncio.run(run_messager_inprocess_async())


def main() -> int:
    if not AGENT_CORE_ROOT.exists():
        print(f"[python] AGENT_CORE_ROOT missing: {AGENT_CORE_ROOT}", file=sys.stderr)
        return 2
    sys.path.insert(0, str(AGENT_CORE_ROOT))
    # openjiuwen 的部分模块会在 CWD 写 logs/ 等运行产物;chdir 到 gitignored
    # 的输出目录,避免污染仓库根。
    OUT.mkdir(parents=True, exist_ok=True)
    os.chdir(OUT)
    shim_logging()

    runners = {
        "stop_condition": run_stop_condition,
        "messager_inprocess": run_messager_inprocess,
    }
    selected = sys.argv[1:] or list(runners)
    for name in selected:
        if name not in runners:
            print(f"[python] unknown seam {name}", file=sys.stderr)
            return 2
        write_outcome(name, runners[name]())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
