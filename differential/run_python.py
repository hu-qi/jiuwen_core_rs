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
    leaf.logger = logger
    sys.modules["openjiuwen.core.common.logging"] = leaf
    utils = types.ModuleType("openjiuwen.core.common.logging.utils")
    utils.set_session_id = lambda _session_id: None
    sys.modules[utils.__name__] = utils
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



def _install_module(name: str, **attributes) -> types.ModuleType:
    """Install a minimal host dependency without replacing the Rail source."""
    parts = name.split(".")
    for index in range(1, len(parts)):
        package = ".".join(parts[:index])
        if package not in sys.modules:
            module = types.ModuleType(package)
            module.__path__ = []
            sys.modules[package] = module
    module = types.ModuleType(name)
    for key, value in attributes.items():
        setattr(module, key, value)
    sys.modules[name] = module
    return module


def shim_llm_retry_dependencies() -> None:
    """Provide only import-time host types needed by the real LLMRetryRail."""
    _install_module(
        "openjiuwen.core.common.exception.codes",
        StatusCode=types.SimpleNamespace(MODEL_CALL_FAILED="MODEL_CALL_FAILED"),
    )
    _install_module(
        "openjiuwen.core.common.exception.errors",
        build_error=lambda _status, error_msg: RuntimeError(error_msg),
    )
    _install_module(
        "openjiuwen.core.single_agent.rail.base",
        AgentCallbackContext=object,
    )
    _install_module(
        "openjiuwen.harness.rails.base",
        DeepAgentRail=type("DeepAgentRail", (), {"__init__": lambda self: None}),
    )


def run_llm_retry() -> dict:
    """Drive the real Python LLMRetryRail's deterministic suffix detector."""
    shim_llm_retry_dependencies()
    rail_module = load_module_file("openjiuwen.harness.rails.llm_retry_rail")
    fixture = load_fixture("llm_retry")
    rail = rail_module.LLMRetryRail(backoff_seconds=fixture["backoff_seconds"])
    cases_out = []
    for case in fixture["cases"]:
        text = case["text"] if "text" in case else case["unit"] * case["count"]
        detected = rail._detect_repeated_suffix(text)
        cases_out.append(
            {
                "name": case["name"],
                "detected": (
                    {"unit": detected[0], "count": detected[1]}
                    if detected is not None
                    else None
                ),
            }
        )
    for item in fixture["error_markers"]:
        message = RuntimeError(item["message"])
        cases_out.append(
            {
                "name": item["name"],
                "repeat": rail._is_repeat_exception(message),
                "timeout": rail._is_stream_timeout_exception(message),
            }
        )
    cases_out.append(
        {
            "name": "backoff_schedule",
            "backoff_ms": [
                round(rail.backoff_delay(index) * 1000)
                for index in fixture["backoff_indexes"]
            ],
        }
    )
    return {"seam": "llm_retry", "cases": cases_out}


def shim_tool_resilience_dependencies() -> None:
    """Provide import-time host types needed by the real tool resilience Rail."""
    _install_module(
        "openjiuwen.core.foundation.tool.schema",
        ToolTimeoutResult=type("ToolTimeoutResult", (), {}),
    )
    _install_module(
        "openjiuwen.core.single_agent.ability_manager",
        AbilityExecutionError=type("AbilityExecutionError", (Exception,), {}),
    )
    _install_module(
        "openjiuwen.core.single_agent.rail.base",
        AgentCallbackContext=object,
        ToolCallInputs=object,
    )
    _install_module(
        "openjiuwen.harness.rails.base",
        DeepAgentRail=type("DeepAgentRail", (), {"__init__": lambda self: None}),
    )


def run_tool_retry() -> dict:
    """Drive the real Python ToolCallResilienceRail classifier."""
    shim_tool_resilience_dependencies()
    rail_module = load_module_file(
        "openjiuwen.harness.rails.tool_call_resilience_rail"
    )
    rail = rail_module.ToolCallResilienceRail()
    fixture = load_fixture("tool_retry")
    exception_types = {
        "TimeoutError": TimeoutError,
        "ConnectionResetError": ConnectionResetError,
        "BrokenPipeError": BrokenPipeError,
        "ConnectionAbortedError": ConnectionAbortedError,
        "ValueError": ValueError,
        "PermissionError": PermissionError,
    }
    cases = []
    for item in fixture["exceptions"]:
        exception_type = exception_types.get(item["type"], RuntimeError)
        cases.append(
            {
                "name": item["name"],
                "retryable": rail._is_retryable_exception(
                    exception_type(item["message"])
                ),
            }
        )
    return {"seam": "tool_retry", "cases": cases}



def shim_task_completion_prompt_dependencies() -> None:
    """Provide host objects while executing the real TaskCompletionRail."""
    class PromptSection:
        def __init__(self, *, name, content, priority):
            self.name = name
            self.content = content
            self.priority = priority

    _install_module(
        "openjiuwen.core.foundation.tool",
        Tool=type("Tool", (), {}),
    )
    _install_module(
        "openjiuwen.core.single_agent.prompts.builder",
        PromptSection=PromptSection,
    )
    sections_module = _install_module(
        "openjiuwen.harness.prompts.sections",
        SectionName=types.SimpleNamespace(COMPLETION_SIGNAL="completion_signal"),
    )
    sections_module.__path__ = []
    _install_module(
        "openjiuwen.core.single_agent.rail.base",
        AgentCallbackContext=object,
        ToolCallInputs=object,
    )
    _install_module(
        "openjiuwen.harness.rails.base",
        DeepAgentRail=type("DeepAgentRail", (), {"__init__": lambda self: None}),
    )


def run_task_completion() -> dict:
    """Drive the real Python TaskCompletionRail prompt hook."""
    shim_task_completion_prompt_dependencies()
    load_module_file("openjiuwen.harness.prompts.sections.task_completion")
    load_module_file("openjiuwen.harness.schema.stop_condition")
    module = load_module_file("openjiuwen.harness.rails.task_completion_rail")
    fixture = load_fixture("task_completion")
    cases = []
    for case in fixture["cases"]:
        sections = []

        class RecordingPromptBuilder:
            language = case["language"]

            def add_section(self, section):
                sections.append(section)

        rail = module.TaskCompletionRail(completion_promise=case["promise"])
        agent = types.SimpleNamespace(system_prompt_builder=RecordingPromptBuilder())
        context = types.SimpleNamespace(agent=agent)
        asyncio.run(rail.before_model_call(context))
        section = sections[0]
        cases.append(
            {
                "name": case["name"],
                "content": section.content[case["language"]],
                "priority": section.priority,
            }
        )
    return {"seam": "task_completion", "cases": cases}



def shim_task_planning_dependencies() -> None:
    """Provide host symbols while executing the real TaskPlanningRail."""
    class PromptSection:
        def __init__(self, *, name, content, priority):
            self.name = name
            self.content = content
            self.priority = priority

    prompts = _install_module(
        "openjiuwen.harness.prompts", PromptSection=PromptSection
    )
    prompts.__path__ = []
    sections = _install_module(
        "openjiuwen.harness.prompts.sections",
        SectionName=types.SimpleNamespace(TODO="todo"),
    )
    sections.__path__ = []
    class Model:
        def __init__(self, model_id):
            self.model_client_config = types.SimpleNamespace(client_id=model_id)
            self.model_config = types.SimpleNamespace(model_name=model_id)

        def __hash__(self):
            return id(self)

        def __eq__(self, other):
            return self is other



    llm = _install_module("openjiuwen.core.foundation.llm", Model=Model)
    llm.__path__ = []
    _install_module("openjiuwen.core.foundation.llm.model", Model=Model)
    _install_module("openjiuwen.core.foundation.tool", ToolCard=object)
    _install_module(
        "openjiuwen.core.runner",
        Runner=types.SimpleNamespace(resource_mgr=types.SimpleNamespace()),
    )
    class TodoStatus:
        IN_PROGRESS = "in_progress"

    _install_module(
        "openjiuwen.harness.schema.task",
        ModelUsageRecord=type("ModelUsageRecord", (), {}),
        TodoItem=type("TodoItem", (), {}),
        TodoStatus=TodoStatus,
    )
    _install_module(
        "openjiuwen.harness.tools",
        TodoTool=type("TodoTool", (), {}),
        TodoCreateTool=type("TodoCreateTool", (), {}),
        TodoListTool=type("TodoListTool", (), {}),
        TodoGetTool=type("TodoGetTool", (), {}),
        TodoModifyTool=type("TodoModifyTool", (), {}),
    )
    _install_module(
        "openjiuwen.harness.workspace.workspace",
        WorkspaceNode=types.SimpleNamespace(TODO="todo"),
    )
    _install_module(
        "openjiuwen.harness.rails.base",
        DeepAgentRail=type("DeepAgentRail", (), {"__init__": lambda self: None}),
    )
    _install_module(
        "openjiuwen.core.single_agent.rail.base",
        AgentCallbackContext=object,
        ToolCallInputs=object,
    )
def run_runtime_model_switching() -> dict:
    """Drive the real Python TaskPlanningRail model switch hook."""
    shim_task_planning_dependencies()
    load_module_file("openjiuwen.harness.prompts.sections.todo")
    module = load_module_file("openjiuwen.harness.rails.task_planning_rail")
    fixture = load_fixture("runtime_model_switching")
    model_class = sys.modules["openjiuwen.core.foundation.llm"].Model
    todo_status = sys.modules["openjiuwen.harness.schema.task"].TodoStatus
    todo_tool_class = sys.modules["openjiuwen.harness.tools"].TodoTool
    cases = []
    for case in fixture["cases"]:
        class TodoTool(todo_tool_class):
            async def load_todos(self, _session_id):
                todo = types.SimpleNamespace(
                    status=todo_status.IN_PROGRESS,
                    selected_model_id=case["selected_model_id"],
                )
                return [todo]

        class Agent:
            def __init__(self, llm):
                self._llm = llm
                self.config = types.SimpleNamespace(model_name=llm.model_config.model_name)

            def set_llm(self, llm):
                self._llm = llm

        default = model_class(case["default_model"])
        models = {
            model_class(model_id): model_id
            for model_id in case["models"]
        }
        rail = module.TaskPlanningRail(model_selection=models)
        rail.system_prompt_builder = types.SimpleNamespace(
            language="en", add_section=lambda _section: None
        )
        rail.tools = [TodoTool()]
        agent = Agent(default)
        context = types.SimpleNamespace(
            agent=agent,
            session=types.SimpleNamespace(get_session_id=lambda: "model-switch"),
        )
        asyncio.run(rail.before_model_call(context))
        cases.append(
            {
                "name": case["name"],
                "selected_model": agent._llm.model_client_config.client_id,
                "config_model": agent.config.model_name,
            }
        )
    return {"seam": "runtime_model_switching", "cases": cases}




def run_task_planning() -> dict:
    """Drive the real Python TaskPlanningRail prompt hook."""
    shim_task_planning_dependencies()
    load_module_file("openjiuwen.harness.prompts.sections.todo")
    module = load_module_file("openjiuwen.harness.rails.task_planning_rail")
    fixture = load_fixture("task_planning")
    cases = []
    for case in fixture["cases"]:
        sections = []

        class RecordingPromptBuilder:
            language = case["language"]

            def add_section(self, section):
                sections.append(section)

        model_class = sys.modules["openjiuwen.core.foundation.llm"].Model
        model_selection = {
            model_class(item["id"]): item["description"]
            for item in case["models"]
        }
        rail = module.TaskPlanningRail(model_selection=model_selection)
        rail.system_prompt_builder = RecordingPromptBuilder()
        asyncio.run(
            rail.before_model_call(
                types.SimpleNamespace(agent=None, session=None)
            )
        )
        section = sections[0]
        cases.append(
            {
                "name": case["name"],
                "content": section.content[case["language"]],
                "priority": section.priority,
            }
        )
    return {"seam": "task_planning", "cases": cases}


def run_team_inbox_fetch() -> dict:
    """Drive real Python ExternalTeamClient fetch and watch paths."""
    class Topic:
        def __init__(self, value):
            self.value = value

        def build(self, session_id, team_name):
            return f"session:{session_id}:team:{team_name}:{self.value}"

    class TeamTopic:
        MESSAGE = Topic("message")
        TASK = Topic("task")

    _install_module("openjiuwen.agent_teams.context", set_session_id=lambda _sid: None, reset_session_id=lambda _token: None)
    _install_module("openjiuwen.agent_teams.external.descriptor", TeamJoinDescriptor=object)
    _install_module("openjiuwen.agent_teams.external.format", render_messages=lambda *args, **kwargs: "", render_task_board=lambda *args, **kwargs: "")
    _install_module("openjiuwen.agent_teams.i18n", set_language=lambda _language: None)
    _install_module("openjiuwen.agent_teams.message_template", expand_message=lambda *args, **kwargs: None)
    _install_module("openjiuwen.agent_teams.messager.base", Messager=object, create_messager=lambda _config: None)
    _install_module("openjiuwen.agent_teams.schema.events", EventMessage=object, TeamTopic=TeamTopic)
    _install_module("openjiuwen.agent_teams.schema.task", TaskCreateResult=object, TaskDetail=object, TaskOpResult=object)
    _install_module("openjiuwen.agent_teams.messager.messager", Messager=object)
    _install_module("openjiuwen.agent_teams.spawn.shared_resources", get_shared_db=lambda _config: None)
    _install_module("openjiuwen.agent_teams.tools.database.engine", get_current_time=lambda: 0)
    _install_module("openjiuwen.agent_teams.tools.models", TeamMember=object, TeamMessageBase=object, TeamTaskBase=object)
    _install_module("openjiuwen.agent_teams.tools.message_manager", TeamMessageManager=object)
    _install_module("openjiuwen.agent_teams.tools.task_manager", TeamTaskManager=object)
    _install_module("openjiuwen.core.common.exception.codes", StatusCode=types.SimpleNamespace(AGENT_TEAM_STATE_INVALID="invalid"))
    _install_module("openjiuwen.core.common.exception.errors", raise_error=lambda *args, **kwargs: (_ for _ in ()).throw(RuntimeError("not connected")))
    _install_module("openjiuwen.core.common.logging", team_logger=logging.getLogger("team-fetch"))
    module = load_module_file("openjiuwen.agent_teams.external.client")
    fixture = load_fixture("team_inbox_fetch")
    cases = []
    for case in fixture["cases"]:
        if case.get("kind") == "watch":
            continue
        class Messages:
            def __init__(self):
                self.marked = []

            async def get_messages(self, **_kwargs):
                return [types.SimpleNamespace(message_id=mid) for mid in case["direct_ids"]]

            async def get_broadcast_messages(self, **_kwargs):
                return [types.SimpleNamespace(message_id=mid) for mid in case["broadcast_ids"]]

            async def mark_message_read(self, message_id, _member_name):
                self.marked.append(message_id)

        class Tasks:
            async def list_tasks(self, **_kwargs):
                return [types.SimpleNamespace(task_id=task_id) for task_id in case["task_ids"]]

        client = module.ExternalTeamClient.__new__(module.ExternalTeamClient)
        client._descriptor = types.SimpleNamespace(member_name="dev-1")
        messages = Messages()
        client._messages = messages
        client._tasks = Tasks()
        view = asyncio.run(client.fetch_inbox(mark_read=case["mark_read"]))
        cases.append({
            "name": case["name"],
            "message_ids": [message.message_id for message in view.messages],
            "task_ids": [task.task_id for task in view.tasks],
            "marked_ids": messages.marked,
        })

    watch_case = next(case for case in fixture["cases"] if case.get("kind") == "watch")

    class WatchMessages:
        def __init__(self):
            self.marked = []

        async def get_messages(self, **_kwargs):
            return [types.SimpleNamespace(message_id=mid) for mid in watch_case["direct_ids"]]

        async def get_broadcast_messages(self, **_kwargs):
            return []

        async def mark_message_read(self, message_id, _member_name):
            self.marked.append(message_id)

    class WatchTasks:
        async def list_tasks(self, **_kwargs):
            return [types.SimpleNamespace(task_id=task_id) for task_id in watch_case["task_ids"]]

    class Messager:
        def __init__(self):
            self.handlers = {}
            self.subscribed = []
            self.unsubscribed = []

        async def subscribe(self, topic, handler):
            self.subscribed.append(topic)
            self.handlers[topic] = handler

        async def unsubscribe(self, topic):
            self.unsubscribed.append(topic)
            self.handlers.pop(topic, None)

    async def exercise_watch():
        client = module.ExternalTeamClient.__new__(module.ExternalTeamClient)
        client._descriptor = types.SimpleNamespace(session_id="s1", team_name="t1", member_name="dev-1")
        messages = WatchMessages()
        messager = Messager()
        client._messages = messages
        client._tasks = WatchTasks()
        client._messager = messager
        observed = []

        async def observer(view):
            observed.append(view)

        task = asyncio.create_task(client.watch(observer))
        for _ in range(10):
            if watch_case["message_topic"] in messager.handlers:
                break
            await asyncio.sleep(0)
        await messager.handlers[watch_case["message_topic"]](None)
        task.cancel()
        try:
            await task
        except asyncio.CancelledError:
            pass
        return {
            "callback_count": len(observed),
            "marked_ids": messages.marked,
            "unsubscribed": sorted(messager.unsubscribed) == sorted(messager.subscribed),
        }

    watch = asyncio.run(exercise_watch())
    cases.append({"name": "watch_cancellation", **watch})
    return {"seam": "team_inbox_fetch", "cases": cases}


def run_team_inbox_format() -> dict:
    """Drive the real Python external inbox formatter."""
    translations = {
        "hitt.silence_note": "stay silent",
        "dispatcher.leader_task_board": "LEADER-BOARD",
        "dispatcher.teammate_task_list": "TEAMMATE-LIST",
        "dispatcher.task_unassigned_marker": " (unassigned)",
        "time.just_now": "刚刚",
        "time.unknown": "unknown time",
    }

    def translate(key, **kwargs):
        template = translations.get(key, key)
        return template.format(**kwargs)

    _install_module(
        "openjiuwen.agent_teams.i18n",
        reply_hint_for=lambda _sender: "please reply",
        t=translate,
    )
    load_module_file("openjiuwen.agent_teams.inbound_render")
    load_module_file("openjiuwen.agent_teams.timefmt")
    module = load_module_file("openjiuwen.agent_teams.external.format")
    fixture = load_fixture("team_inbox_format")
    cases = []
    for case in fixture["cases"]:
        if case["kind"] == "message":
            message = types.SimpleNamespace(
                broadcast=case["broadcast"],
                timestamp=case["timestamp"],
                from_member_name=case["from_member_name"],
                message_id=case["message_id"],
                content=case["content"],
            )
            output = module.render_message(
                message,
                is_human_agent=case["is_human_agent"],
                now_ms=case["now_ms"],
                body=case["body"],
            )
        elif case["kind"] == "task_board":
            tasks = [types.SimpleNamespace(**task) for task in case["tasks"]]
            output = module.render_task_board(tasks, is_leader=case["is_leader"], now_ms=case["now_ms"])
        else:
            raise ValueError(f"unknown team inbox case kind {case['kind']}")
        cases.append({"name": case["name"], "output": output})
    return {"seam": "team_inbox_format", "cases": cases}


def shim_prompt_attachment_dependencies() -> None:
    """Provide narrow host shims for the real attachment manager."""
    _install_module(
        "openjiuwen.core.context_engine.base",
        ContextWindow=object,
        ModelContext=object,
    )
    _install_module(
        "openjiuwen.core.foundation.llm",
        BaseMessage=object,
        UserMessage=type("UserMessage", (), {"__init__": lambda self, **kwargs: None}),
    )


def _prompt_attachment_view(item) -> dict | None:
    if item is None:
        return None
    kind = item.kind.value if hasattr(item.kind, "value") else str(item.kind)
    return {
        "id": item.id,
        "section": item.section,
        "kind": kind,
        "content": item.content,
        "priority": item.priority,
        "source": item.source,
        "session_id": item.session_id,
        "expires_at": item.expires_at,
        "metadata": item.metadata,
        "content_kind": item.content_kind,
        "content_sha256": item.content_sha256,
    }


async def _run_prompt_attachment_case(module, case: dict) -> dict:
    manager = module.PromptAttachmentManager()
    session_id = case["session_id"]
    ids = {}
    operations = []
    for op in case["operations"]:
        name = op["op"]
        if name == "add":
            item = await manager.add_section(
                session_id=session_id,
                section=op["section"],
                content=op["content"],
                kind=op["kind"],
                source=op["source"],
                priority=op["priority"],
                metadata=op.get("metadata"),
                expires_at=op.get("expires_at"),
            )
            ids[op["name"]] = item.id
            operations.append({"name": "add", "item": _prompt_attachment_view(item)})
        elif name == "update":
            item = await manager.update_by_id(
                ids[op["name"]],
                module.PromptAttachmentUpdate(
                    content=op["content"],
                    metadata=op["metadata"],
                ),
            )
            operations.append({"name": "update", "item": _prompt_attachment_view(item)})
        elif name == "list":
            items = await manager.list_by_filter(
                session_id=session_id,
                section=op.get("section"),
            )
            operations.append({
                "name": "list",
                "items": [_prompt_attachment_view(item) for item in items],
            })
        elif name == "collect":
            items = await manager.collect_for_session(session_id)
            operations.append({
                "name": "collect",
                "items": [_prompt_attachment_view(item) for item in items],
            })
        elif name == "clear_section":
            count = await manager.clear_section(session_id=session_id, section=op["section"])
            operations.append({"name": "clear_section", "count": count})
        elif name == "clear_session":
            count = await manager.clear_session(session_id)
            operations.append({"name": "clear_session", "count": count})
        else:
            raise ValueError(f"unknown prompt attachment operation {name}")
    return {"name": case["name"], "operations": operations}


def run_prompt_attachment() -> dict:
    """Drive the real Python PromptAttachmentManager against the fixture."""
    shim_prompt_attachment_dependencies()
    module = load_module_file("openjiuwen.harness.prompts.prompt_attachment_manager")
    fixture = load_fixture("prompt_attachment")
    cases = [
        asyncio.run(_run_prompt_attachment_case(module, case))
        for case in fixture["cases"]
    ]
    return {"seam": "prompt_attachment", "cases": cases}


def shim_goal_manager_dependencies() -> None:
    """Provide narrow host shims while executing the real GoalManager."""
    class EventManager:
        def discard_goal_work(self, *, session_id, goal_id):
            return None

        def has_running_goal(self, *, goal_id):
            return False

        def push_goal(self, work):
            return False

    class InteractionEvent:
        @staticmethod
        def goal_updated(payload):
            return payload

    class RoundWorkItem:
        @staticmethod
        def goal(**kwargs):
            return kwargs

    _install_module(
        "openjiuwen.harness.task_loop.event_manager",
        EventManager=EventManager,
    )
    _install_module(
        "openjiuwen.harness.schema.interaction",
        InteractionEvent=InteractionEvent,
        RoundWorkItem=RoundWorkItem,
    )


def _goal_record_view(record) -> dict | None:
    if record is None:
        return None
    assessment = record.last_assessment
    return {
        "objective": record.objective,
        "status": record.status.value,
        "revision": record.revision,
        "attempt_count": record.attempt_count,
        "token_usage": record.token_usage.to_dict(),
        "max_attempts": record.max_attempts,
        "token_budget": record.token_budget,
        "last_assessment": assessment.to_dict() if assessment else None,
        "last_stop_reason": record.last_stop_reason,
    }


async def _run_goal_manager_case(module, case: dict) -> dict:
    class Store:
        session_id = case["session_id"]

        def __init__(self):
            self.record = None

        def load(self):
            return self.record

        def save(self, record):
            self.record = record

        def clear(self):
            self.record = None

        async def commit(self):
            return None

    store = Store()
    manager = module.GoalManager(
        store=store,
        event_manager=module.EventManager(),
        control_lock=asyncio.Lock(),
        has_output_stream=lambda: False,
        cancel_active_round=lambda **kwargs: asyncio.sleep(0),
        emit_event=lambda _event: None,
        notify_work=lambda: None,
    )
    goal_id = None
    revision = 0
    operations = []
    schema = sys.modules["openjiuwen.harness.goal.schema"]
    for op in case["operations"]:
        name = op["op"]
        if name == "set":
            try:
                record = await manager.set(
                    op["objective"],
                    max_attempts=op.get("max_attempts"),
                    token_budget=op.get("token_budget"),
                )
            except schema.GoalOperationError as error:
                operations.append({"name": "set_error", "code": error.code})
            else:
                goal_id = record.goal_id
                revision = record.revision
                operations.append({"name": "set", "record": _goal_record_view(record)})
        elif name in ("begin", "stale_begin"):
            record = await manager.begin_attempt(
                goal_id=goal_id or "",
                revision=revision if name == "begin" else max(0, revision - 1),
            )
            if record is not None:
                revision = record.revision
            operations.append({"name": name, "record": _goal_record_view(record)})
        elif name == "usage":
            await manager.accumulate_usage(
                goal_id=goal_id or "",
                revision=revision,
                input_tokens=op.get("input_tokens", 0),
                output_tokens=op.get("output_tokens", 0),
                cached_input_tokens=op.get("cached_input_tokens", 0),
            )
            operations.append(
                {"name": "usage", "record": _goal_record_view(await manager.get())}
            )
        elif name in ("pause", "resume"):
            record = await getattr(manager, name)()
            if record is not None and name == "resume":
                revision = record.revision
            operations.append({"name": name, "record": _goal_record_view(record)})
        elif name in ("continue", "complete", "block"):
            status = {
                "continue": schema.GoalAssessmentStatus.CONTINUE,
                "complete": schema.GoalAssessmentStatus.COMPLETE,
                "block": schema.GoalAssessmentStatus.BLOCKED,
            }[name]
            record = await manager.apply_assessment(
                goal_id=goal_id or "",
                revision=revision,
                assessment=schema.GoalAssessment(
                    status=status,
                    evidence=op.get("evidence", ""),
                    remaining_work=op.get("remaining_work"),
                ),
            )
            operations.append({"name": name, "record": _goal_record_view(record)})
        elif name == "clear":
            record = await manager.clear()
            operations.append({"name": "clear", "record": _goal_record_view(record)})
        else:
            raise ValueError(f"unknown goal operation {name}")
    return {"name": case["name"], "operations": operations}


def run_goal_manager() -> dict:
    """Drive the real Python GoalManager against the shared fixture."""
    shim_goal_manager_dependencies()
    load_module_file("openjiuwen.harness.goal.schema")
    module = load_module_file("openjiuwen.harness.goal.manager")
    fixture = load_fixture("goal_manager")
    cases = [asyncio.run(_run_goal_manager_case(module, case)) for case in fixture["cases"]]
    return {"seam": "goal_manager", "cases": cases}

async def _run_task_lifecycle_case(case: dict) -> dict:
    # shim_logging pre-seeds ``openjiuwen``; restore its filesystem path for
    # the full team package while retaining the lightweight common logger.
    openjiuwen = sys.modules.get("openjiuwen")
    if openjiuwen is not None:
        openjiuwen.__path__ = [str(AGENT_CORE_ROOT / "openjiuwen")]
    agent_teams = sys.modules.get("openjiuwen.agent_teams")
    if agent_teams is not None:
        agent_teams.__path__ = [str(AGENT_CORE_ROOT / "openjiuwen" / "agent_teams")]
    core = sys.modules.get("openjiuwen.core")
    if core is not None:
        core.__path__ = [str(AGENT_CORE_ROOT / "openjiuwen" / "core")]
    for package, relative in {
        "openjiuwen.agent_teams": "agent_teams",
        "openjiuwen.agent_teams.schema": "agent_teams/schema",
        "openjiuwen.agent_teams.tools": "agent_teams/tools",
    }.items():
        module = sys.modules.get(package) or types.ModuleType(package)
        module.__path__ = [str(AGENT_CORE_ROOT / "openjiuwen" / relative)]
        sys.modules[package] = module
    # Mutation parity does not depend on transport/event serialization. Keep
    # the real task manager and DAO code, but avoid importing the aggregate
    # messager/workflow package initializers.
    events = types.ModuleType("openjiuwen.agent_teams.schema.events")
    class _Event:
        def __init__(self, **kwargs): self.__dict__.update(kwargs)
    events.EventMessage = type("EventMessage", (), {"from_event": staticmethod(lambda event: event)})
    for event_name in ("TaskCancelledEvent", "TaskClaimedEvent", "TaskCompletedEvent", "TaskCreatedEvent", "TaskListDrainedEvent", "TaskPlanRequestEvent", "TaskPlanResponseEvent", "TaskReleasedEvent", "TaskReviewVoteEvent", "TaskRevisionRequestedEvent", "TaskRevokedEvent", "TaskStartedEvent", "TaskSubmittedForReviewEvent", "TaskUnblockedEvent", "TaskUpdatedEvent", "TaskVerifiedEvent"):
        setattr(events, event_name, _Event)
    events.TeamTopic = type("TeamTopic", (), {"TASK": type("TaskTopic", (), {"build": staticmethod(lambda *_args: "task")})})
    sys.modules[events.__name__] = events
    messager = types.ModuleType("openjiuwen.agent_teams.messager")
    messager.Messager = object
    sys.modules[messager.__name__] = messager
    message_manager = types.ModuleType("openjiuwen.agent_teams.tools.message_manager")
    message_manager.TeamMessageManager = object
    sys.modules[message_manager.__name__] = message_manager
    from unittest.mock import AsyncMock

    from openjiuwen.agent_teams.context import reset_session_id, set_session_id
    from openjiuwen.agent_teams.schema.status import MemberMode
    from openjiuwen.agent_teams.tools.database import DatabaseConfig, DatabaseType, TeamDatabase
    from openjiuwen.agent_teams.tools.task_manager import TeamTaskManager
    token = set_session_id("task-lifecycle-diff")
    database = TeamDatabase(DatabaseConfig(db_type=DatabaseType.SQLITE, connection_string=":memory:"))
    bus = AsyncMock()
    try:
        await database.initialize()
        await database.team.create_team(team_name="task-team", display_name="Task Team", leader_member_name="leader")
        for member in ("m1", "m2"):
            await database.member.create_member(
                member_name=member,
                team_name="task-team",
                display_name=member,
                agent_card="{}",
                status="BUSY",
                mode=MemberMode.BUILD_MODE.value,
            )
        author = TeamTaskManager(team_name="task-team", member_name="m1", db=database, messager=bus)
        reviewer = TeamTaskManager(team_name="task-team", member_name="m2", db=database, messager=bus)
        operations = []
        for op in case["operations"]:
            name = op["op"]
            if name == "create":
                result = await author.add(title=op["title"], content=op["content"], task_id=op["task_id"])
                if result.ok and op.get("reviewers"):
                    await author.set_reviewer(op["task_id"], op["reviewers"])
                operations.append({"name": "create", "ok": result.ok, "status": (result.task.status if result.ok else None)})
            elif name == "update":
                result = await author.update_task(op["task_id"], title=op.get("title"), content=op.get("content"))
                operations.append({"name": "update", "ok": result.ok})
            elif name in ("dependency", "dependency_cycle"):
                result = await author.add_dependencies(op["task_id"], op["depends_on"])
                operations.append({"name": name, "ok": result.ok})
            elif name == "claim":
                result = await author.claim(op["task_id"])
                operations.append({"name": "claim", "ok": result.ok})
            elif name == "complete":
                result = await author.complete(op["task_id"])
                task = await author.get(op["task_id"])
                operations.append({"name": "complete", "ok": result.ok, "status": task.status})

            elif name == "review":
                result = await reviewer.verify_task(op["task_id"], op["decision"])
                task = await author.get(op["task_id"])
                operations.append({"name": "review", "ok": result.ok, "status": task.status})
            else:
                raise ValueError(f"unknown task operation {name}")
        tasks = await author.list_tasks()
        operations.append({"name": "final", "tasks": [{"id": t.task_id, "title": t.title, "content": t.content, "status": t.status, "assignee": t.assignee} for t in tasks]})
        return {"name": case["name"], "operations": operations}
    finally:
        await database.close()
        reset_session_id(token)


def run_task_lifecycle() -> dict:
    fixture = load_fixture("task_lifecycle")
    return {"seam": "task_lifecycle", "cases": [asyncio.run(_run_task_lifecycle_case(case)) for case in fixture["cases"]]}

def run_subagents() -> dict:
    """Drive the real Python browser capability resolver."""
    module = load_module_file("openjiuwen.harness.tools.browser_move.playwright_runtime.browser_capabilities")
    fixture = load_fixture("subagents")
    cases = []
    for case in fixture["cases"]:
        resolved = module.resolve_browser_capabilities(case["requested"])
        cases.append({
            "name": case["name"],
            "requested": list(resolved.requested_names),
            "selected": list(resolved.selected_names),
            "rejected": list(resolved.rejected_names),
            "allowed": list(resolved.allowed_tool_names),
        })
    return {"seam": "subagents", "cases": cases}


def shim_cancellation_rail_dependencies() -> None:
    """Provide narrow import seams while loading the real cancellation rail."""
    rail_base = types.ModuleType("openjiuwen.core.single_agent.rail.base")

    class AgentCallbackContext:
        pass

    rail_base.AgentCallbackContext = AgentCallbackContext
    sys.modules[rail_base.__name__] = rail_base

    harness_base = types.ModuleType("openjiuwen.harness.rails.base")

    class DeepAgentRail:
        def __init__(self) -> None:
            pass

    harness_base.DeepAgentRail = DeepAgentRail
    sys.modules[harness_base.__name__] = harness_base


async def _run_cancellation_callback_case(module: types.ModuleType, case: dict) -> dict:
    class Orchestrator:
        should_cancel = case["cancelled"]

    class Context:
        def __init__(self) -> None:
            self.force_finish = None

        def request_force_finish(self, result: dict) -> None:
            self.force_finish = result

    rail = module.CancellationRail()
    rail.bind(Orchestrator())
    context = Context()
    if case["phase"] == "after_model_call":
        await rail.after_model_call(context)
    elif case["phase"] == "before_tool_call":
        await rail.before_tool_call(context)
    else:
        raise ValueError(f"unknown cancellation phase {case['phase']}")
    return {
        "name": case["name"],
        "phase": case["phase"],
        "requested": context.force_finish is not None,
        "cancelled": bool(context.force_finish and context.force_finish.get("cancelled")),
    }


def run_cancellation_callback() -> dict:
    """Drive the real Python CancellationRail at model/tool checkpoints."""
    shim_cancellation_rail_dependencies()
    module = load_module_file("openjiuwen.rsi.auto_harness.rails.cancellation_rail")
    fixture = load_fixture("cancellation_callback")
    return {
        "seam": "cancellation_callback",
        "cases": [asyncio.run(_run_cancellation_callback_case(module, case)) for case in fixture["cases"]],
    }


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
        "tool_retry": run_tool_retry,
        "llm_retry": run_llm_retry,
        "task_completion": run_task_completion,
        "task_planning": run_task_planning,
        "prompt_attachment": run_prompt_attachment,
        "runtime_model_switching": run_runtime_model_switching,
        "team_inbox_format": run_team_inbox_format,
        "team_inbox_fetch": run_team_inbox_fetch,
        "goal_manager": run_goal_manager,
        "task_lifecycle": run_task_lifecycle,
        "subagents": run_subagents,
        "cancellation_callback": run_cancellation_callback,
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
