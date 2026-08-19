# Rust 1:1 移植规格:resources 扩展加载与解析(extension_loader / extension_resolver)

> 本文档是 `agent-core/openjiuwen/harness/resources/` 下 Python 源码的移植规格,目标是 Rust 侧 1:1 行为对齐(路径语义、字段默认值、错误消息、优先级规则)。
> 覆盖文件:
> - `extension_loader.py`(1005 行)
> - `extension_resolver.py`(285 行)
> - `__init__.py`(56 行)
> - `builtin_rules.yaml`(84 行)
>
> 同时引用(为准确描述构造的 Spec 字段/默认值):`schema/extension_spec.py`、`schema/deep_agent_spec.py`、`schema/build_context.py`、`schema/config.py`、`core/foundation/tool/mcp/base.py`、`core/single_agent/prompts/builder.py`、`core/single_agent/schema/agent_card.py`、`core/common/schema/card.py`。
>
> 行号以当前仓库文件为准,标注格式为 `文件:行号`(如 `L140-162` 表示 extension_loader.py 第 140–162 行;其他文件会写明文件名)。

---

## 0. 总览

本模块做两件事:

1. **包加载(loader)**:把 Plugin / AgentTemplate 包(目录或 manifest 文件)解析为 Spec 对象(`PluginSpec` / `AgentTemplateSpec`)。路径在此阶段全部绝对化。
2. **解析(resolver)**:把 Spec 物化为运行时零件(`ExtensionParts`),即把声明变为活对象(Tool / McpServerConfig / AgentRail / PromptSection / ResolvedSkill / SubAgentConfig),但不绑定到 host Agent(绑定由 `extension_binder.apply_extension_hot` 完成)。

数据流:

```
find_*_manifest(path) ──> manifest 绝对路径
load_*_package(manifest_path) ──> PluginSpec / AgentTemplateSpec(路径全绝对)
resolve_*_parts(spec, ctx) ──> ExtensionParts
```

`__init__.py`(`__init__.py:L34-56`)只做 re-export,`__all__` 见 `extension_loader.py:L999-1005`、`extension_resolver.py:L276-285`、`__init__.py:L34-56`。公开 API 面:

- loader: `find_agent_template_manifest`、`find_plugin_manifest`、`load_agent_template_package`、`load_plugin_package`、`normalize_package_mcps`
- resolver: `ExtensionParts`、`LoadRecord`、`ResolvedPromptSection`、`ResolvedSkill`、`ResourceKind`、`ResourceRef`、`resolve_agent_template_parts`、`resolve_plugin_parts`
- schema 类型(re-export): `AgentTemplateSpec`、`McpDirSpec`、`McpServerSpec`、`MemorySpec`、`PluginSpec`、`PromptSectionSpec`、`RubricSpec`、`SkillSpec`

---

## 1. 数据模型(extension_spec.py,需 Rust 直接建模)

> 这些类定义在 `schema/extension_spec.py`(全文 217 行)。loader 构造它们、resolver 消费它们。**移植时必须复刻其字段、默认值与校验规则。**

### 1.1 `_ExtensionSpecModel`(基类,L38-41)
- `model_config = ConfigDict(extra="forbid")` → **未知键一律拒绝**(pydantic ValidationError)。Rust 侧:反序列化时对未知字段报错。
- 例外:`RubricSpec`、`MemorySpec` 覆盖为 `extra="allow"`(L101、L107),即任意 dict 都接受、未知键原样保留。

### 1.2 `McpServerSpec`(L44-67)

| 字段 | 类型 | 默认值 |
|---|---|---|
| `type` | Literal["stdio","sse","streamable_http"] | `"stdio"` |
| `server_name` | str \| None | `None` |
| `server_id` | str \| None | `None` |
| `url` | str \| None | `None` |
| `command` | str | `""` |
| `args` | list[str] | `[]` |
| `env` | dict[str,str] | `{}` |
| `cwd` | str \| None | `None` |
| `params` | dict[str,Any] | `{}`(validator:L64-67 强制"纯数据",见 1.8) |
| `auth_headers` | dict[str,str] | `{}` |
| `auth_query_params` | dict[str,str] | `{}` |

### 1.3 `SkillSpec`(L76-81)
- `dir: str`(必填)、`mode: Literal["all","auto_list"] = "all"`、`enabled_skills: list[str] | None = None`。

### 1.4 `PromptSectionSpec`(L84-95)
- `name: str`(必填)、`content: dict[str,str]`(必填)、`priority: int = 100`、`render_params: dict[str,Any] = {}`(validator 纯数据)。
- **注意默认优先级是 100**,不是 10/30(10/30 只出现在 legacy 归一化里,见 2.16)。

### 1.5 `RubricSpec` / `MemorySpec`(L98-107)
- 空模型 + `extra="allow"`:`model_validate(任意 dict)` 恒成功,字段原样保留。

### 1.6 `AgentTemplateSpec`(L110-143)
| 字段 | 类型 | 默认 |
|---|---|---|
| `agent_card` | `AgentCard` | 必填 |
| `model` | `ModelSpec \| None` | `None` |
| `prompt_sections` | `list[PromptSectionSpec]` | `[]` |
| `tools` | `list[BuiltinToolSpec]` | `[]` |
| `mcps` | `list[McpServerSpec]` | `[]` |
| `rails` | `list[RailSpec]` | `[]` |
| `skills` | `list[SkillSpec]` | `[]` |
| `memories` | `list[MemorySpec]` | `[]` |
| `rubrics` | `list[RubricSpec]` | `[]` |
| `subagents` | `list[AgentTemplateSpec]` | `[]` |
| `metadata` | `dict[str,Any]` | `{}`(validator 纯数据,L140-143) |

### 1.7 `PluginSpec`(L146-169)
- `id: str`(**必填**)、`name: str|None`、`description: str|None`、`prompt_sections`、`tools`、`mcps`、`rails`、`skills`、`metadata`(后六项默认同上)。

### 1.8 `_plain_data` 递归校验(L17-30)—— `params`/`render_params`/`metadata` 的值域约束
- 通过:`None`、`bool`、`int`、`float`、`str`。
- `list`:递归校验每项。
- `dict`:每个键必须是 `str`,否则 `ValueError("mapping keys must be strings")`;递归校验每个值。
- 其他类型:`ValueError(f"{type(value).__name__} is not JSON/YAML serializable data")`。
- 校验只抛错、不修改值(`_validate_plain_mapping` L33-35 包装后返回原值)。

### 1.9 `validate_plugin_paths(spec)`(L180-204)—— 路径绝对性校验(纯函数,可 1:1)
- 输入:`PluginSpec | AgentTemplateSpec`;无返回值;不满足则抛 `ValueError`。
- 规则(仅当值为真值时才检查):
  - 每个 `tools` 项,若 `type == "harness.tool.file"` 且 `params.file_path` 非空 → 必须绝对,字段名 `"tools.params.file_path"`。
  - 每个 `rails` 项,若 `type == "harness.rail.file"` 且 `params.file_path` 非空 → 字段名 `"rails.params.file_path"`。
  - 每个 `mcps` 项,若 `cwd` 非空 → 字段名 `"mcps.cwd"`。
  - 每个 `skills` 项 → `dir` 必须绝对,字段名 `"skills.dir"`。
  - 递归进 `spec.subagents`(仅直接子级,子级再递归)。
- 判断:`Path(value).expanduser().is_absolute()`,失败消息(L172-177):
  `f"{field_name} must already be an absolute path (the package loader or the caller of load_plugin_spec is responsible for absolutizing it): {value!r}"`

### 1.10 引用的外部模型(loader/resolver 构造它们,Rust 需等价类型)
- `AgentCard`(`core/single_agent/schema/agent_card.py:L16-28`,继承 `BaseCard`,`core/common/schema/card.py:L9-19`):`id: str`(缺省 `uuid4().hex`,**非确定性**)、`name: str = ""`、`description: str = ""`、`input_params/output_params/interface_url` 默认 None。`BaseCard` 无 `extra` 配置 → pydantic 默认 `extra="ignore"`(未知键静默丢弃)。
- `BuiltinToolSpec`(`deep_agent_spec.py:L286-328`):`type: str`、`params: dict[str,Any] = {}`;`build(*, language, tool_id=None, context=None)` 走 `_TOOL_PROVIDER_REGISTRY`(L320-328),未知 type 抛 `ValueError(f"Unknown tool type '{self.type}'. Registered types: {list(_TOOL_PROVIDER_REGISTRY)}")`。
- `RailSpec`(`deep_agent_spec.py:L245-283`):`type: str`、`params: dict = {}`;`build(*, language, workspace=None, context=None)` 走 `_RAIL_PROVIDER_REGISTRY`(L275-283),未知 type 抛 `ValueError(f"Unknown rail type '{self.type}'. Registered types: {list(_RAIL_PROVIDER_REGISTRY)}")`。
- `ModelSpec = TeamModelConfig`(`deep_agent_spec.py:L116-133`):`model_client_config`(必填)、`model_request_config: Optional[...] = None`。
- `SubAgentSpec`(`deep_agent_spec.py:L336-353`):`agent_card`(必填)、`system_prompt`(必填)、`tools: list[ToolCard | BuiltinToolSpec] = []`、`mcps: list[McpServerConfig] = []`、`model: Optional[TeamModelConfig] = None`、`rails: Optional[list[RailSpec]] = None`、`skills: Optional[list[str]] = None`、`workspace/sys_operation/language/prompt_mode/max_iterations/factory_name/factory_kwargs` 等。
- `BuildContext`(`schema/build_context.py:L26-61`):dataclass,非 pydantic。字段 `language: str = "cn"`、`member_name/role/workspace/member_card_id/project_dir: Optional[...] = None`、`extras: dict = {}`;有 `derive(**overrides)` 浅拷贝方法。resolver 只读 `ctx.language`、`ctx.workspace`、`ctx.extras`。
- `SubAgentConfig`(`schema/config.py:L290-311`):运行时子代理配置(dataclass),resolver 的 `ExtensionParts.subagents` 元素类型。
- `McpServerConfig`(`core/foundation/tool/mcp/base.py:L44-51`):`server_id`(缺省 `uuid4().hex`,非确定性)、`server_name`(必填)、`server_path`(必填)、`client_type: str = 'sse'`、`params/auth_headers/auth_query_params`。
- `PromptSection`(`core/single_agent/prompts/builder.py:L21-43`):`name`、`content: dict[str,str]`、`priority: int = 100`;`render(language="cn")` 与 `char_count`。
- `Tool` / `ToolCard`(`core/foundation/tool/`):运行时工具对象(由 provider build 产生,环境相关)。

---

## 2. extension_loader.py 规格

### 2.1 模块常量(L26-101)

| 常量 | 值 | 行号 |
|---|---|---|
| `_CANONICAL_PLUGIN_YAML_FIELDS` | `{"schema_version","id","source","name","description","tools","mcps","rails","prompt_sections","skills","metadata"}`(11 键,frozenset) | L28-42 |
| `_MANIFEST_NAMES` | `("harness_config.yaml", "expert_harness.yaml", "harness.yaml")`(顺序即优先级) | L44 |
| `_PACKAGE_MANIFEST_NAME` | `"manifest.json"` | L45 |
| `_SUBAGENT_MANIFEST_NAME` | `".subagent.json"` | L46 |
| `_LEGACY_PACKAGE_MANIFEST_NAMES` | `("harness_config.yaml", "harness.yaml")`(重合并路径;`expert_harness.yaml` 走轻路径) | L50 |
| `_MCP_DIR_MANIFEST_NAMES` | `("mcps.json", "mcps.yaml")`(顺序即优先级) | L51 |
| `_MCP_TRANSPORT_ALIASES` | `{"stdio":"stdio","sse":"sse","http":"streamable_http","streamable-http":"streamable_http","streamable_http":"streamable_http"}` | L52-58 |
| `_LEGACY_AGENT_CONTROL_KEYS` | 21 个键:`add_general_purpose_agent, completion_timeout, context, default_mode, enable_async_subagent, enable_task_loop, enable_task_planning, language, max_iterations, meta, permissions, progressive_tool_always_visible_tools, progressive_tool_default_visible_tools, progressive_tool_enabled, progressive_tool_max_loaded_tools, prompt_mode, restrict_to_work_dir, stop_eval_conditions, workspace` | L59-79 |
| `_LEGACY_TOOL_TO_RAIL` | 21 个别名(见 2.18) | L82-101 |

### 2.2 `find_plugin_manifest(path: str | Path) -> Path`(L140-162)
语义:定位 `load_plugin_package` 要用的 manifest。**先 `Path(path).expanduser()`**(展开 `~`,依赖 `$HOME`)。

- 若为目录:
  1. 存在 `manifest.json` → 直接返回 `manifest_json.resolve()`(**不看 packageType**,类型错误由 `load_plugin_package` 拒绝;一旦有 manifest.json 就不再回退 legacy YAML)。
  2. 否则 → `_find_legacy_yaml_manifest(dir)`。
- 若非目录:
  - 若 `name == "manifest.json"`:
    - 文件不存在 → `FileNotFoundError(f"Plugin manifest not found: {requested_path}")`。
    - 存在 → `requested_path.resolve()`。
  - 否则 → `_find_legacy_yaml_manifest(path)`。

`_find_legacy_yaml_manifest(path)`(L104-123,内部 helper):
- 目录:按 `_MANIFEST_NAMES` 顺序找第一个 `is_file()` 的,返回 `resolve()`;都没有 → `FileNotFoundError(f"{' or '.join(_MANIFEST_NAMES)} not found: {requested_path}")`(即 `"harness_config.yaml or expert_harness.yaml or harness.yaml not found: <path>"`)。
- 文件:若 `name not in _MANIFEST_NAMES` 或 `not is_file()` → 同上消息;否则 `resolve()`。

### 2.3 `find_agent_template_manifest(path) -> Path`(L165-180)
- **只接受 manifest.json,无 legacy 回退**。
- 目录:存在 `manifest.json` → `resolve()`;否则 `FileNotFoundError(f"manifest.json not found: {requested_path}")`。
- 文件:若 `name != "manifest.json"` 或 `not is_file()` → 同上消息;否则 `resolve()`。

### 2.4 `load_plugin_package(manifest_path) -> PluginSpec`(L183-193)
- `manifest = Path(manifest_path).expanduser().resolve(strict=True)` → **路径必须存在**,否则 `FileNotFoundError`(OSError)。
- `manifest.name == "manifest.json"` → `_load_plugin_manifest_json`;否则 → `_load_plugin_legacy_yaml`。
- 返回的 Spec 上所有资源路径都是绝对路径。

### 2.5 `load_agent_template_package(manifest_path) -> AgentTemplateSpec`(L196-235)
1. `manifest = Path(manifest_path).expanduser().resolve(strict=True)`。
2. `manifest.name != "manifest.json"` → `ValueError(f"AgentTemplate manifest must be named 'manifest.json': {manifest}")`(注意用 `!r` 渲染常量 → 消息含引号)。
3. `package_root = manifest.parent.resolve(strict=True)`。
4. `payload = _read_json_mapping(manifest)`。
5. `payload.get("packageType") != "agent_template"` → `ValueError(f"{manifest} declares packageType={payload.get('packageType')!r}, expected 'agent_template'")`。
6. `agent_card_payload = payload.get("agentCard")`,falsy → `ValueError(f"AgentTemplate manifest {manifest} is missing required 'agentCard'")`。
7. `base_dir = manifest.parent`;`subagents` 列表(见 2.6)。
   - **陷阱**:comprehension 直接写 `item["dir"]`(L214-215)—— 元素缺 `"dir"` 键抛 `KeyError`,元素为字符串抛 `TypeError`,都不是友好的 ValueError。
8. 构造 `AgentTemplateSpec`:
   - `agent_card=AgentCard.model_validate(agent_card_payload)`(未知键忽略;缺 id 时自动 `uuid4().hex`)。
   - `model=_build_model_spec(payload.get("model"), base_dir, package_root)`。
   - `prompt_sections=_build_persona_prompt_sections(payload.get("persona"), ...)`。
   - `tools/_build_tool_specs(payload.get("tools"), ...)`、`mcps/_build_mcp_specs(...)`、`rails/_build_rail_specs(...)`、`skills/_build_skill_specs(...)`。
   - `memories=[MemorySpec.model_validate(item) for item in _as_list(payload.get("memories"))]`、`rubrics` 同理。
   - `subagents`、`metadata=dict(payload.get("metadata") or {})`。

### 2.6 `_load_agent_subtemplate(subagent_dir, *, package_root) -> AgentTemplateSpec`(L296-326)
- manifest 固定为 `subagent_dir / ".subagent.json"`;不存在 → `FileNotFoundError(f".subagent.json not found for subagent template: {subagent_dir}")`。
- `payload` 含 `"subagents"` 键 → `ValueError(f"Subagent template {manifest} must not declare 'subagents' (only the root agent_template may declare direct subagents)")`。
- `agent_name = payload.get("agentName")`,falsy → `ValueError(f"Subagent template {manifest} is missing required 'agentName'")`。
- `agent_card = AgentCard(id=str(agent_name), name=str(agent_name), description=_first_display_description(payload.get("displayDescription")))`。
- 返回 `AgentTemplateSpec`(只有 agent_card/model/prompt_sections/tools/mcps/rails/skills 七项,**无** memories/rubrics/subagents/metadata → 默认空)。

`_first_display_description(value) -> str`(L329-339):非 dict → `""`;取 `value["en"]` → 其次 `value["zh"]` → 否则第一个真值文本 → 全无 `""`。

### 2.7 `_read_json_mapping(path) -> dict`(L342-346)
`json.loads(path.read_text(encoding="utf-8"))`;顶层非 dict → `ValueError(f"Manifest must contain a mapping: {path}")`。

### 2.8 `_resolve_new_manifest_path(raw_path, *, base_dir, package_root, must_be_dir) -> Path`(L349-370)—— 新格式包内路径约束
1. `Path(raw_path).expanduser().is_absolute()` → `ValueError(f"manifest path must be package-relative, got absolute path: {raw_path!r}")`(注意:`~/x` 展开后也算绝对,被拒;但 `~` 本身作为相对段不展开,属边界行为)。
2. `candidate = (base_dir / raw_path).resolve(strict=True)` → 不存在抛 OSError/FileNotFoundError。
3. `not candidate.is_relative_to(package_root)` → `ValueError(f"manifest path {raw_path!r} escapes package root {package_root}")`。
4. `must_be_dir and not candidate.is_dir()` → `ValueError(f"manifest path {raw_path!r} must be a directory: {candidate}")`;`not must_be_dir and not candidate.is_file()` → `ValueError(f"manifest path {raw_path!r} must be a file: {candidate}")`。
5. 返回 `candidate`。

> 移植提示:`resolve(strict=True)` = 规范化 + 解析符号链接 + 必须存在;Rust 可用 `fs::canonicalize` 近似(但 canonicalize 会解析全部符号链接且要求存在,语义一致);`is_relative_to` 需按组件前缀比较(**注意 `..` 已在 resolve 中消化**)。

### 2.9 `_build_persona_prompt_sections(persona, *, base_dir, package_root) -> list[PromptSectionSpec]`(L373-390)
- falsy → `[]`;非 dict 或缺 `"dir"` → `ValueError(f"persona must be a mapping with 'dir': {persona!r}")`。
- `persona_dir = _resolve_new_manifest_path(..., must_be_dir=True)`。
- `markdown_files = sorted((c for c in persona_dir.rglob("*.md") if c.is_file()), key=lambda c: c.relative_to(persona_dir).as_posix())` — **递归找所有 .md(含隐藏文件),按相对 posix 路径字符串排序**(确定性关键点)。
- 空 → `ValueError(f"persona dir {persona_dir} has no markdown files")`。
- `text = "\n\n".join(md.read_text(encoding="utf-8") for md in markdown_files)`(每文件内容以空行 `\n\n` 拼接)。
- 返回 `[PromptSectionSpec(name="identity", content={"cn": text, "en": text}, priority=10)]`。

### 2.10 `_build_model_spec(model_ref, *, base_dir, package_root) -> ModelSpec | None`(L393-406)
- falsy → `None`;非 dict 或缺 `"file"` → `ValueError(f"model must be a mapping with 'file': {model_ref!r}")`。
- `model_path = _resolve_new_manifest_path(..., must_be_dir=False)`;`payload = _read_json_mapping(model_path)`。
- `payload.get("model") is None` → `ValueError(f"model file {model_path} is missing the outer 'model' key")`。
- `ModelSpec.model_validate(model_payload)`。

### 2.11 `_build_tool_specs / _build_rail_specs`(L409-424 / L427-442)
- 逐项:`_as_list(items)`;非 dict 或缺 `"file"` → `ValueError(f"tool entry must be a mapping with 'file': {item!r}")`(rail 版消息把 `tool` 换成 `rail`)。
- `file_path = _resolve_new_manifest_path(..., must_be_dir=False)`;`class_name = item.get("class") or item.get("class_name")`。
- 产出 `BuiltinToolSpec(type="harness.tool.file", params={"file_path": str(file_path), "class_name": class_name})`(rail 版 `type="harness.rail.file"`)。**注意 `params` 恒含 `class_name` 键(值为 None 也保留)**。

### 2.12 `_build_skill_specs`(L445-461)+ `_skill_mode`(L464-467)+ `_skill_enabled_list`(L470-475)
- 元素可为字符串 → 转为 `{"dir": raw_item}`;否则须为 dict 且含 `"dir"`,否则 `ValueError(f"skill entry must be a mapping with 'dir': {raw_item!r}")`。
- `directory = _resolve_new_manifest_path(..., must_be_dir=True)`。
- `SkillSpec(dir=str(directory), mode=_skill_mode(item.get("mode", "all")), enabled_skills=_skill_enabled_list(item.get("enabled_skills")))`。
- `_skill_mode`:值必须在 `("all","auto_list")`,否则 `ValueError(f"skill mode must be 'all' or 'auto_list', got {value!r}")`。
- `_skill_enabled_list`:None → None;list → `[str(i) for i in value]`;否则 `ValueError(f"enabled_skills must be a list, got {value!r}")`。

### 2.13 `_build_mcp_specs(items, *, base_dir, package_root) -> list[McpServerSpec]`(L478-491)
- 逐项非 dict → `ValueError(f"mcp entry must be a mapping: {item!r}")`。
- 若 `"dir" in item` 且 `not _looks_like_mcp_server_entry(item)` → 目录引用:`mcp_dir = _resolve_new_manifest_path(..., must_be_dir=True)`;对 `_load_mcp_dir_entries(mcp_dir)` 返回的每个原始条目调用 `_build_mcp_server_spec`。
- 否则直接 `_build_mcp_server_spec(item, ...)`。

`_looks_like_mcp_server_entry(item) -> bool`(L722-734):`item` 含以下任一键即视为 server 条目:`"type","transport","server_name","name","command","url","server_id"`。

`_load_mcp_dir_entries(mcp_dir) -> list[dict]`(L494-502):
- 按 `_MCP_DIR_MANIFEST_NAMES`(`mcps.json` → `mcps.yaml`)顺序找第一个存在文件;`payload = _load_data_file(path)`;若 dict → `servers = payload.get("servers", payload.get("mcps", []))`,否则 `servers = payload`;返回 `[item for item in _as_list(servers) if isinstance(item, dict)]`(**原始条目,不做字段归一化**)。
- 都没有 → `FileNotFoundError(f"MCP dir {mcp_dir} must contain one of: mcps.json, mcps.yaml")`。

### 2.14 `_build_mcp_server_spec(item, *, base_dir, package_root) -> McpServerSpec`(L505-524)
1. `entry = _normalize_mcp_server_entry(item)`;为 None(被禁用)→ `ValueError(f"mcp entry is disabled and cannot be used: {item!r}")`。
2. cwd 处理(注意:`entry` 已被归一化,`cwd` 是原样字符串或不存在):
   - 有 `cwd`:`cwd_path = Path(raw_cwd).expanduser()`;绝对 → `ValueError(f"mcp cwd must be package-relative, got absolute path: {raw_cwd!r}")`;`candidate = (base_dir / cwd_path).resolve()`(**非 strict,不要求存在**);越界 → `ValueError(f"mcp cwd {raw_cwd!r} escapes package root {package_root}")`;`entry["cwd"] = str(candidate)`。
   - 无 `cwd` → `entry["cwd"] = str(base_dir)`。
3. `McpServerSpec.model_validate(entry)`。

### 2.15 `_normalize_mcp_server_entry(item) -> dict | None`(L764-809)—— 关键归一化函数(纯函数,无 FS)
1. `raw = dict(item)`。
2. `"enabled" in raw and not bool(raw.pop("enabled"))` → 返回 None(禁用)。
   - **陷阱**:`bool()` 真值语义 —— YAML `enabled: false` → False → 禁用;`enabled: "false"`(字符串)→ 真 → 仍启用;`0`、`""`、`None`、`[]` → 禁用。
3. `transport = str(raw.pop("transport", raw.pop("type", "stdio")) or "stdio").strip().lower()`。
   - **陷阱**:Python 先求值默认参数 → `raw.pop("type", "stdio")` 总会执行,即 `type` 键无论是否被用都被移除;`transport` 存在则优先,否则用 `type` 值,都没有 → `"stdio"`。
4. `client_type = _MCP_TRANSPORT_ALIASES.get(transport)`;None → `ValueError(f"Unsupported MCP transport/type: {transport!r}")`。
5. 字段弹出与默认:
   - `server_name = raw.pop("server_name", None) or raw.pop("name", None)`
   - `server_id = raw.pop("server_id", None)`;`url = raw.pop("url", None)`
   - `command = str(raw.pop("command", "") or "")`
   - `args = raw.pop("args", None) or []`;`env = raw.pop("env", None) or {}`;`cwd = raw.pop("cwd", None)`
   - `params = dict(raw.pop("params", None) or {})`
   - `auth_headers = dict(raw.pop("auth_headers", None) or raw.pop("headers", None) or {})`(兼容别名 `headers`)
   - `auth_query_params = dict(raw.pop("auth_query_params", None) or {})`
   - `timeout_s = raw.pop("timeout_s", None)`;若非 None 且 `params` 无 `"timeout_s"` → `params["timeout_s"] = int(timeout_s)`(不可转 int 抛 ValueError)。
   - `raw.pop("description", None)`、`raw.pop("dir", None)`(忽略残余元数据键)。
6. 产出固定键集:
   ```
   {"type": client_type,
    "server_name": str(server_name).strip() if server_name else None,
    "command": command,
    "args": [str(v) for v in args] if isinstance(args, list) else [],
    "env": {str(k): str(v) for k, v in dict(env).items()},
    "params": params,
    "auth_headers": {str(k): str(v) for k, v in auth_headers.items()},
    "auth_query_params": {str(k): str(v) for k, v in auth_query_params.items()}}
   ```
   - `server_id` 非 None 且 `str(server_id).strip()` 非空 → 加 `"server_id"`。
   - `url` 非 None 且 strip 非空 → 加 `"url"`。
   - `cwd` 非 None 且 strip 非空 → 加 `"cwd"`。
7. 返回该 dict(供 `McpServerSpec.model_validate`;键集与 schema 完全匹配,故 extra="forbid" 不会触发)。

### 2.16 `_load_plugin_manifest_json(manifest) -> PluginSpec`(L238-263)
1. `payload = _read_json_mapping(manifest)`。
2. `packageType != "plugin"` → `ValueError(f"{manifest} declares packageType={package_type!r}, expected 'plugin'")`。
3. 禁用键 `("persona","agentCard","model","subagents","memories","rubrics")`:任一存在 → `ValueError(f"Plugin manifest {manifest} must not declare {forbidden_key!r}")`。
4. `plugin_id = payload.get("id")`,falsy → `ValueError(f"Plugin manifest {manifest} is missing required 'id'")`。
5. `package_root = base_dir.resolve(strict=True)`。
6. 构造 `PluginSpec(id=str(plugin_id), name=..., description=..., prompt_sections=[PromptSectionSpec.model_validate(i) for i in _as_list(payload.get("prompt_sections"))], tools=_build_tool_specs(payload.get("tools"),...), mcps=_build_mcp_specs(...), rails=..., skills=..., metadata=dict(payload.get("metadata") or {}))`。
   - 注意新格式的 prompt_sections 是**内联内容**(`name/content/priority/render_params`),不走 persona 目录。

### 2.17 `_load_plugin_legacy_yaml(manifest) -> PluginSpec`(L266-293)
1. `payload = _normalize_legacy_plugin_yaml(manifest)`;`_remap_legacy_tools_to_rails(payload)`。
2. `plugin_id = str(payload.get("id") or package_dir.name)`(id 缺省取包目录名)。
3. 构造 PluginSpec:
   - `prompt_sections=[PromptSectionSpec.model_validate(i) for i in payload.get("prompt_sections", [])]`
   - `tools=[BuiltinToolSpec.model_validate(_absolutize_legacy_resource(i, package_dir, kind="tool")) for i in payload.get("tools", [])]`
   - `mcps=[McpServerSpec.model_validate(_absolutize_legacy_mcp(i, package_dir)) for i in payload.get("mcps", [])]`
   - `rails=[RailSpec.model_validate(_absolutize_legacy_resource(i, package_dir, kind="rail")) for i in payload.get("rails", [])]`
   - `skills=[SkillSpec.model_validate(_absolutize_legacy_skill(i, package_dir)) for i in payload.get("skills", [])]`
   - `metadata=dict(payload.get("metadata") or {})`

### 2.18 `_normalize_legacy_plugin_yaml(manifest) -> dict`(L561-606)—— 三分支
1. `payload = _load_yaml_mapping(manifest)`(见 2.21);`package_dir = manifest.parent`。
2. **分支 A**:`payload.get("schema_version") == "expert_harness.v1"` → 浅拷贝,仅 `mcps = _normalize_mcps(mcps, package_dir)`,直接返回(不做其他合并/归一化)。
3. **分支 B**:`manifest.name not in _LEGACY_PACKAGE_MANIFEST_NAMES`(即文件是 `expert_harness.yaml`)→ 浅拷贝;若含 `"mcps"` → 归一化;直接返回。
4. **分支 C**(`harness_config.yaml` / `harness.yaml`,完整合并管线):
   a. `resources = normalized.pop("resources", None)`;若为 dict → 对 `("tools","mcps","rails","prompt_sections","skills")` 逐个 `_merge_items(normalized, key, resources.get(key))`。
   b. `_merge_legacy_prompt_sections(normalized)`(2.19)。
   c. `_drop_legacy_agent_control_fields(normalized)`(2.20)。
   d. `normalized["schema_version"] = "expert_harness.v1"`。
   e. `normalized.setdefault("id", str(normalized.get("name") or package_dir.name))`;`normalized.setdefault("name", package_dir.name)`。
   f. `_merge_sidecar_prompt_sections(normalized, package_dir)`(2.22)。
   g. 列表文件合并(2.23):`tools/tools.yaml`→"tools";`mcps/mcps.yaml`→"mcps";`mcps/mcps.json`→"mcps";`rails/rails.yaml`→"rails";`skills/skills.yaml`→"skills"。
   h. `normalized["skills"] = _normalize_skills(normalized.get("skills", []))`。
   i. `normalized["mcps"] = _normalize_mcps(normalized.get("mcps"), package_dir)`。
   j. `normalized["tools"] = _normalize_resource_items(..., kind="tool", package_dir)`;`rails` 同理 kind="rail"(2.24)。
   k. 最终过滤:**只保留 `_CANONICAL_PLUGIN_YAML_FIELDS` 中的键**(丢弃 role/version 等杂项;注意 `file_sections` 也会被丢弃 → legacy `prompts.sections[].file` 形式实际不进入 PluginSpec)。

### 2.19 legacy prompt 合并
`_merge_legacy_prompt_sections(payload)`(L645-657):`prompts = payload.pop("prompts", None)`;非 dict 则返回;对 `_as_list(prompts.get("sections"))` 中每个 dict:`normalized = _normalize_legacy_prompt_section(section)`;若含 `"filename"` → 并入键 `"file_sections"`(随后被 2.18-k 丢弃),否则并入 `"prompt_sections"`。

`_normalize_legacy_prompt_section(section) -> dict`(L660-680):
- `content = _normalize_legacy_section_content(section.get("content"))`;`render_params = dict(section.get("render_params") or {})`。
- `file = section.get("file")` 非 None → `{"filename": str(file), "content": content, "render_params": render_params}`。
- 否则:`name = str(section["name"])`(缺键 → KeyError);`priority = section.get("priority")`,None 时 `10 if name == "identity" else 30`;返回 `{"name": name, "content": content, "priority": priority, "render_params": render_params}`。

`_normalize_legacy_section_content(content)`(L683-690):None → `{}`;str → `{"cn": s, "en": s}`;dict → `{str(k): str(v) for k, v in content.items()}`;其他 → `{}`。

### 2.20 `_remap_legacy_tools_to_rails(payload)`(L609-637)—— 纯函数
- 收集现有 rails 的 type 集合(仅 dict 且 `type` 非 None 者)。
- 遍历 tools:字符串 → `raw_type = tool`;dict 且有 `type` → `str(tool["type"])`;否则原样保留。
- `rail_type = _LEGACY_TOOL_TO_RAIL.get(raw_type)`:
  - None → 工具保留;
  - 命中且 `rail_type` 不在 rails 集合 → 追加 `{"type": rail_type, "params": {}}` 并登记;该工具被**移除**。
- 映射表(L82-101):`todo/core.todo→core.task_planning`、`lsp/core.lsp→core.lsp`、`ask_user/core.ask_user/ask_user_tool/core.ask_user_tool→core.ask_user`、`filesystem/core.filesystem/shell/core.shell/bash/core.bash/powershell/core.powershell/code/core.code→core.sys_operation`。
- 结果写回 `payload["tools"]`、`payload["rails"]`。

`_drop_legacy_agent_control_fields(payload)`(L640-642):对 `_LEGACY_AGENT_CONTROL_KEYS` 逐个 `pop(key, None)`。

### 2.21 数据文件读取(L693-704)
- `_load_yaml_mapping(path)`:读文件;顶层非 dict → `ValueError(f"Harness manifest must contain a mapping: {path}")`。
- `_load_data_file(path)`:utf-8 读取;`.json` 后缀 → `json.loads`;否则 `yaml.safe_load(text) or {}`(空 YAML → `{}`)。
  - **移植注意**:Python `yaml.safe_load` 是 YAML 1.1 语义(`yes/no/on/off` → bool、`1:2` 键等);Rust serde_yaml 是 YAML 1.2 语义(`yes` 是字符串)。这会影响 `enabled: yes` 之类输入(见 2.15 真值陷阱),需显式处理或记录差异。

### 2.22 侧边 prompt 文件(L812-835)
`_merge_sidecar_prompt_sections(payload, package_dir)`:
- 固定对 `(("identity.md","identity",10), ("soul.md","soul",20))`:文件存在 → 并入 `"prompt_sections"` 的 `{"name": name, "content": {"cn": text, "en": text}, "priority": priority}`。
- `package_dir/"prompt_sections"/"sections.yaml"` 存在 → `_load_yaml_mapping(...).get("sections", [])`,每个 dict → `_merge_items(payload, "prompt_sections", _normalize_prompt_section(section, package_dir))`。

`_normalize_prompt_section(section, package_dir)`(L838-846):拷贝;`file` 键弹出 → `content = {"cn": read_text, "en": read_text}`(经 `_resolve_section_file`);否则若 `content` 是 str → 转双语文案。

`_resolve_section_file(package_dir, file_name)`(L849-856):绝对路径 → 直接用(不检查存在);否则 `package_dir/path` 存在则用;否则 `package_dir/"prompt_sections"/"files"/path`。

### 2.23 列表文件合并(L859-865)
`_merge_list_file(payload, path, key)`:文件不存在 → 返回;`data = _load_data_file(path)`;若 dict → `data = data.get(key, data.get("servers", []))`;`_merge_items(payload, key, data)`。

`_merge_items(payload, key, value)`(L868-872):None → 返回;否则 `payload.setdefault(key, []).extend(_as_list(value))`(追加语义,允许重复)。

`_as_list(value)`(L875-880):None → `[]`;list → 原样;其他 → `[value]`。

### 2.24 legacy 资源项归一化(L903-996)
- `_normalize_resource_items(items, *, kind, package_dir)`(L903-907):对每项调用 `_normalize_resource_item` 并展平(结果可能是 list)。
- `_normalize_resource_item(item, *, kind, package_dir)`(L917-956):
  - 字符串 → `{"type": f"core.{item}", "params": {}}`。
  - 非 dict → 原样返回。
  - `type == "builtin"` → `_normalize_builtin_resource_item`(L959-968):`params = _pop_legacy_params(item)`;`names = item.pop("names", None)`,若 None 且 `item.get("name")` 非 None → `[item.pop("name")]`;对 `_as_list(names)` 每个 name 产出 `{"type": f"core.{name}", "params": dict(params)}`(names 为空 → 空列表)。
  - `type == "entry_point"` → `_normalize_entry_point_resource_item`(L971-976):`params = _pop_legacy_params(item)`;`name = item.pop("name", None)` 非 None → `params.setdefault("name", name)`;产出 `{"type": f"harness.{kind}.entry_point", "params": params}`。
  - `"type" in item and item["type"] != "package"` → `_normalize_params`(L979-982):拷贝 + `params = _pop_legacy_params`。
  - 否则(无 type 或 type=="package"):
    - `params = _pop_legacy_params(normalized)`;`file_name = normalized.pop("file", None)`。
    - 有 `file` → `params.setdefault("file_path", file_name)`;`"class"` → `params.setdefault("class_name", normalized.pop("class"))`;`"class_name"` 同理;产出 `{"type": f"harness.{kind}.file", "params": params}`。
    - 否则若 `_has_module_class_resource_spec(normalized, params)`(L910-914:`normalized["module"]` 是 str 且(含 `class`/`class_name` 键或 `params.class_name` 非 None)):
      - `module_name = str(normalized.pop("module"))`;`class_name` 依次取 `normalized.pop("class_name", None)` → `normalized.pop("class", None)` → `params.pop("class_name", None)`。
      - `file_path = _module_to_package_file(module_name, package_dir)`(L991-996):前缀 `f"openjiuwen.extensions.harness.{package_dir.name}."` 不匹配 → None;匹配 → `f"{relative.replace('.', '/')}.py"`(**返回相对路径字符串**)。
      - file_path None → `params.setdefault("import_path", f"{module_name}.{class_name}")`,产出 `{"type": f"harness.{kind}.import", "params": params}`;否则 `params.setdefault("file_path", file_path); params.setdefault("class_name", class_name)`,产出 `{"type": f"harness.{kind}.file", "params": params}`。
    - 否则**返回原始 `item`**(不做任何变换)。
- `_pop_legacy_params(item)`(L985-988):`params = dict(item.pop("params", None) or {})`;`params.update(dict(item.pop("kwargs", None) or {}))`。
- `_normalize_skill(s)`(L887-890):str → `{"dir": item}`;其他原样。
- `_normalize_skills(items)`(L893-900):dict 且 `dirs` 为 list → 展平成多个 `{"dir": v}`;否则 `_normalize_skill`。

### 2.25 legacy 路径绝对化(L527-558)
- `_absolutize_legacy_resource(item, *, package_dir, kind)`(L527-534):非 dict 或 `item.get("type") != f"harness.{kind}.file"` → 原样;否则 `params = dict(item.get("params") or {})`;`params["file_path"]` 非空 → 替换为 `str(_resolve_legacy_path(str(file_path), package_dir))`;返回 `{**item, "params": params}`。
- `_absolutize_legacy_skill(item, *, package_dir)`(L537-541):`dir` 非空 → `{**item, "dir": str(_resolve_legacy_path(...))}`。
- `_absolutize_legacy_mcp(item, *, package_dir)`(L544-551):非 dict → 原样;无 `cwd` → `{**item, "cwd": str(package_dir.resolve())}`;有 → `{**item, "cwd": str(_resolve_legacy_path(...))}`。
- `_resolve_legacy_path(raw_path, package_dir)`(L554-558):`candidate = Path(raw_path).expanduser()`;相对 → `package_dir / candidate`;`candidate.resolve(strict=True)`(不存在 → FileNotFoundError)。**注意 legacy 路径允许绝对(不拒绝),仅 new-manifest 拒绝绝对**。

### 2.26 `normalize_package_mcps(mcps, package_dir) -> list[dict]`(L126-132,公开函数)
- 即 `_normalize_mcps(mcps, Path(package_dir).expanduser().resolve())` 的公开包装,供外部(AgentTemplate 包仅指向 MCP 目录等场景)使用。
- `_normalize_mcps(items, package_dir)`(L707-719):逐项非 dict → 跳过;`"dir" in item` 且 `not _looks_like_mcp_server_entry(item)` → `normalized.extend(_load_mcps_from_dir(package_dir, str(item["dir"])))`;否则 `entry = _normalize_mcp_server_entry(item)`,非 None → 追加。
- `_load_mcps_from_dir(package_dir, relative_dir)`(L737-761):路径解析(相对 → `(package_dir / rel).resolve()`,绝对 → `resolve()`);找 `mcps.json`/`mcps.yaml` 第一个;payload dict → `payload.get("servers", payload.get("mcps", []))`;逐项 dict → `_normalize_mcp_server_entry`,非 None 追加;都没有 → `FileNotFoundError(f"MCP dir {mcp_dir} must contain one of: mcps.json, mcps.yaml")`。
  - 与 `_load_mcp_dir_entries`(2.13)的差异:后者不归一化条目、由 `_build_mcp_server_spec` 逐个处理;前者立即归一化。

---

## 3. extension_resolver.py 规格

### 3.1 类型(需 Rust 建模)

| 类型 | 定义 | 说明 |
|---|---|---|
| `ResourceKind(str, Enum)` | L41-49 | 枚举 `TOOL="tool"`、`MCP="mcp"`、`RAIL="rail"`、`PROMPT_SECTION="prompt_section"`、`SKILL="skill"`、`SUBAGENT="subagent"` |
| `ResourceRef(BaseModel)` | L52-57 | `kind: ResourceKind`、`identity: str`、`extra: dict = {}` |
| `LoadRecord(BaseModel)` | L60-65 | `load_id: str`(缺省 `uuid4().hex`,非确定性)、`source_uri: str|None`、`refs: list[ResourceRef] = []` |
| `ResolvedSkill`(frozen dataclass) | L68-74 | `directory: str`、`mode: Literal["all","auto_list"]`、`enabled_skills: list[str]|None = None` |
| `ResolvedPromptSection`(frozen dataclass) | L77-88 | `section: PromptSection`、`replace_existing: bool = False`(仅 AgentTemplate 根 persona `identity` 为 True;Plugin 恒 False) |
| `ExtensionParts`(dataclass) | L91-100 | `tools: list[Tool|ToolCard]`、`mcps: list[McpServerConfig]`、`rails: list[AgentRail]`、`prompt_sections: list[ResolvedPromptSection]`、`skills: list[ResolvedSkill]`、`subagents: list[SubAgentConfig]`,全部默认 `[]` |

> resolver 本身不产生 `LoadRecord`/`ResourceRef`(那是 binder 的记账类型),但作为公共 API re-export。

### 3.2 `resolve_plugin_parts(spec: PluginSpec, ctx: BuildContext) -> ExtensionParts`(L103-116)
1. `validate_plugin_paths(spec)`(1.9)。
2. `language = ctx.language or "cn"`。
3. 返回 `ExtensionParts`:
   - `tools=_resolve_tools(spec.tools, language, ctx)`
   - `mcps=[_build_mcp_server_config(m) for m in spec.mcps]`
   - `rails=_resolve_rails(spec.rails, language, ctx)`
   - `prompt_sections=[ResolvedPromptSection(section=_resolve_prompt_section(s, ctx), replace_existing=False) for s in spec.prompt_sections]`
   - `skills=[_resolve_skill(s) for s in spec.skills]`
   - `subagents=[]`(默认;**Plugin 永不产生子代理**)。

### 3.3 `resolve_agent_template_parts(spec: AgentTemplateSpec, ctx: BuildContext) -> ExtensionParts`(L119-150)
1. `validate_plugin_paths(spec)`。
2. `language = ctx.language or "cn"`;`parent_model = (ctx.extras or {}).get("_parent_model")`。
3. `spec.subagents` 非空且 `parent_model is None` → `ValueError("AgentTemplate subagent resolve requires BuildContext.extras['_parent_model']")`。
4. 子代理物化:对每个 child:
   - `subagent_spec = _adapt_agent_template_to_subagent_spec(child, ctx)`;
   - `built = subagent_spec.build(parent_model=parent_model, language=language, context=ctx)`;
   - 非 `SubAgentConfig` 实例 → `TypeError(f"SubAgentSpec.build returned {type(built).__name__}, expected SubAgentConfig")`。
5. 返回 ExtensionParts,与 plugin 版相同,但 `prompt_sections` 的 `replace_existing=True`,并带 `subagents`。

### 3.4 `_adapt_agent_template_to_subagent_spec(template, ctx) -> SubAgentSpec`(L153-172)
- `ordered_sections = sorted(template.prompt_sections, key=lambda item: item.priority)`(稳定排序,优先级升序)。
- `rendered = [_render_prompt_section_text(s, ctx, language) for s in ordered_sections]`。
- 返回 `SubAgentSpec(agent_card=template.agent_card, system_prompt="\n\n".join(rendered), tools=list(template.tools), mcps=[_build_mcp_server_config(m) for m in template.mcps], model=template.model, rails=list(template.rails) or None, skills=[skill.dir for skill in template.skills] or None, language=language)`。
- 后续 `SubAgentSpec.build(...)` 是环境相关调用(模型构造等)。

### 3.5 物化 helper
- `_resolve_tools(specs, *, language, ctx)`(L175-179):逐项 `tool_spec.build(language=language, context=ctx)`,`_as_built_list` 展平。
- `_resolve_rails(specs, *, language, ctx)`(L182-186):逐项 `rail_spec.build(language=language, workspace=ctx.workspace, context=ctx)`,展平。
- `_as_built_list(built)`(L189-194):None → `[]`;list → 过滤掉 None 项;其他 → `[built]`。
- `_resolve_skill(spec)`(L241-242):`ResolvedSkill(directory=spec.dir, mode=spec.mode, enabled_skills=spec.enabled_skills)`(纯函数)。
- `_resolve_prompt_section(spec, ctx)`(L230-233):`params = _render_params(spec.render_params, ctx)`;`content = {lang: _render_template(text, params) for lang, text in spec.content.items()}`(保留 dict 键序);`PromptSection(name=spec.name, content=content, priority=spec.priority)`。
- `_render_prompt_section_text(spec, ctx, language)`(L236-238):`_select_content(spec.content, language)` 后 `_render_template`。

### 3.6 `_build_mcp_server_config(spec: McpServerSpec) -> McpServerConfig`(L197-227)—— 纯数据变换,可 1:1
- `params = dict(spec.params or {})`。
- `spec.type == "stdio"`:
  - `spec.command` 非空 → `params.setdefault("command", spec.command)`;`spec.args` 非空 → `params.setdefault("args", list(spec.args))`;`spec.env` → `params.setdefault("env", dict(spec.env))`;`spec.cwd` → `params.setdefault("cwd", spec.cwd)`。
  - `server_path = spec.command or spec.server_name or "stdio"`。
- 否则:`server_path = spec.url or spec.command`。
- `config_kwargs = {"server_name": spec.server_name or spec.command or "mcp_server", "server_path": server_path, "client_type": spec.type, "params": params, "auth_headers": dict(spec.auth_headers or {}), "auth_query_params": dict(spec.auth_query_params or {})}`。
- `spec.server_id` 非空 → 加 `"server_id"`。
- `McpServerConfig(**kwargs)`(`server_id` 未提供时自动 `uuid4().hex`)。

### 3.7 模板渲染(L38、L257-273)—— 纯函数,可 1:1
- `_TEMPLATE_PATTERN = re.compile(r"{{\s*([a-zA-Z_][a-zA-Z0-9_]*)\s*}}")`(L38)。
- `_render_template(text, params)`(L266-273):替换所有匹配;键不在 params → **保留原文**(`match.group(0)`);键在 → `str(params[key])`。
- `_render_params(render_params, ctx)`(L257-263):基础 dict `{"language": ctx.language or "cn", "workspace": getattr(ctx.workspace, "root_path", None) or ""}`,再 `update(dict(render_params or {}))`(用户参数覆盖默认)。
- `_select_content(content, language)`(L245-254):优先 `content[language]` → `content["en"]` → `content["cn"]` → 第一个值 → `""`。

---

## 4. builtin_rules.yaml(不参与 loader/resolver 逻辑)

- 位置:`resources/builtin_rules.yaml`(84 行)。**extension_loader.py / extension_resolver.py 完全不引用它**。
- 消费方:`openjiuwen/harness/security/tiered_policy.py`(`_package_builtin_rules_path` L64-65、`get_builtin_security_rules` 等,合并进内置 guardrail 层、位于用户 permissions 之前)。移植属于权限引擎范畴,不在本规格主线上。
- 结构:顶层 `rules:` 列表,每条含:
  - `id: str`
  - `description: str`
  - `tools: list[str]`(本文件固定为 `[bash, mcp_exec_command, create_terminal]`)
  - `match_type: "command"`
  - `pattern: str`(以 `re:` 前缀标注正则,`(?i)` 忽略大小写;见下文)
  - `severity: "CRITICAL"`(13 条中 12 条)或 `action: "deny"`(1 条:`shell_system_shutdown_or_reboot` L79-84)
- 规则清单(文件内顺序):
  1. `shell_fs_recursive_or_forced_delete`(L16-21):rm -rf/-fr/--recursive/--force、find -delete、shred、del、rd /s /q
  2. `shell_disk_partition_or_raw_device_write`(L23-28):mkfs/mke2fs/fdisk/parted/diskpart/format、dd of=/dev/、`> /dev/sd*` 等
  3. `shell_download_and_execute`(L30-35):curl|bash、iwr|iex、bash < <(curl)
  4. `shell_obfuscated_or_dynamic_execution`(L37-42):base64 -d|bash、certutil -decode、-EncodedCommand、Convert::FromBase64String、eval、iex、python -c 等
  5. `shell_reverse_shell_or_bind_shell`(L44-49):/dev/tcp、nc -e、socat EXEC、bash -i、python socket 等
  6. `shell_privilege_escalation`(L51-56):sudo/su/doas/pkexec/runas/Start-Process -Verb RunAs/psexec/schtasks /ru SYSTEM
  7. `shell_data_exfiltration`(L58-63):curl --data/-F/-T、scp/rsync/sftp/ftp、nc 高危端口
  8. `shell_remote_execution_or_lateral_movement`(L65-70):Invoke-Command/Enter-PSSession/winrs/wmic process call create/psexec/ssh "cmd"
  9. `shell_fork_bomb_or_resource_abuse`(L72-77):fork bomb、kill -9 -1、ulimit -u unlimited
  10. `shell_system_shutdown_or_reboot`(L79-84):shutdown/reboot/halt/poweroff、init/telinit 0/6 —— **此条无 severity,带 `action: deny`**
- 移植注意:正则是 Python `re` 语法(支持 `(?i)` 内联标志、命名分组等)。Rust 侧若用 `regex` crate,`(?i)` 兼容,但部分结构(如 `(?i)` 出现在 `re:` 前缀内部)需按"去掉 `re:` 前缀后把 `(?i)` 转为大小写不敏感匹配"处理。**本文件与 loader/resolver 无调用关系,此处仅记录结构供安全引擎移植参考。**

---

## 5. 确定性 vs 环境依赖划分(核心交付点)

### A. 纯函数 / 确定性 —— 可 1:1 移植,优先实现(输入相同 → 输出相同,无 IO)

**extension_loader.py:**
| 功能 | 行号 | 备注 |
|---|---|---|
| `_as_list` | L875-880 | 单元素包装 |
| `_section_content` | L883-884 | `{"cn": s, "en": s}` |
| `_normalize_legacy_section_content` | L683-690 | 文案归一化 |
| `_normalize_legacy_prompt_section` | L660-680 | 优先级 10/30 规则 |
| `_merge_legacy_prompt_sections` | L645-657 | dict 变换 |
| `_merge_items` | L868-872 | 追加合并 |
| `_drop_legacy_agent_control_fields` | L640-642 | 删键 |
| `_remap_legacy_tools_to_rails` | L609-637 | 工具→rail 重映射 |
| `_looks_like_mcp_server_entry` | L722-734 | 键探测 |
| `_normalize_mcp_server_entry` | L764-809 | **核心归一化**(含 transport 别名、enabled 真值、timeout_s) |
| `_MCP_TRANSPORT_ALIASES` 等常量 | L26-101 | 静态数据 |
| `_normalize_skill` / `_normalize_skills` | L887-900 | dirs 展开 |
| `_normalize_resource_item` 及其子函数 | L903-996 | legacy 资源项归一化(内部调用 `_module_to_package_file`,后者为纯字符串运算) |
| `_first_display_description` | L329-339 | en/zh/首个值 |
| `_skill_mode` / `_skill_enabled_list` | L464-475 | 校验器 |
| `_has_module_class_resource_spec` | L910-914 | 判断器 |

**extension_resolver.py:**
| 功能 | 行号 | 备注 |
|---|---|---|
| `_TEMPLATE_PATTERN` | L38 | 正则常量 |
| `_render_template` | L266-273 | `{{key}}` 替换,未知键保留 |
| `_select_content` | L245-254 | 语言回退链 |
| `_resolve_skill` | L241-242 | 直传 |
| `_as_built_list` | L189-194 | 展平/去 None |
| `_build_mcp_server_config` | L197-227 | McpServerSpec→McpServerConfig 纯变换(除缺省 uuid) |
| `_resolve_prompt_section` / `_render_prompt_section_text` | L230-238 | 给定 ctx 后纯 |
| `_render_params` | L257-263 | 见下方环境项 |
| `_adapt_agent_template_to_subagent_spec` | L153-172 | 数据装配(不含 `.build()` 调用) |
| `validate_plugin_paths` + `_require_absolute_path` + `_plain_data`(extension_spec.py) | L180-204 / L172-177 / L17-35 | 见下方 HOME 注意项 |

### B. 文件系统依赖 —— Rust 侧需真实路径实现(或抽象为可注入的 FS trait)

| 功能 | 行号 | 依赖的 FS 操作 |
|---|---|---|
| `find_plugin_manifest` / `find_agent_template_manifest` / `_find_legacy_yaml_manifest` | L140-180 / L104-123 | `is_dir`、`is_file`、`resolve`(非 strict) |
| `load_plugin_package` / `load_agent_template_package` 入口 | L190 / L199-202 | `resolve(strict=True)` |
| `_read_json_mapping` / `_load_data_file` / `_load_yaml_mapping` | L342-346 / L700-704 / L693-697 | 文件读取 + JSON/YAML 解析 |
| `_resolve_new_manifest_path` | L349-370 | `resolve(strict=True)`、`is_dir`、`is_file`、`is_relative_to` |
| `_build_persona_prompt_sections` | L373-390 | `rglob("*.md")`、`is_file`、`read_text`;排序键保证输出确定性 |
| `_build_model_spec` | L393-406 | 文件读取 + JSON |
| `_load_mcp_dir_entries` / `_load_mcps_from_dir` | L494-502 / L737-761 | 目录探测 + 文件读取 |
| `_build_mcp_server_spec` 的 cwd 段 | L513-521 | `resolve`(非 strict) + `is_relative_to` |
| `_merge_sidecar_prompt_sections` / `_merge_list_file` / `_resolve_section_file` | L812-856 / L859-865 | `is_file`、`read_text`、存在性探测 |
| `_absolutize_legacy_*` / `_resolve_legacy_path` | L527-558 | `resolve(strict=True)`(存在性) |
| `expanduser`(所有 `Path(...).expanduser()` 调用) | 多处 | 读 `$HOME` 环境变量 |

> Rust 侧建议:定义一个 `Fs` trait(`is_dir/is_file/read_text/canonicalize/read_dir_recursive`),loader 通过它访问文件系统,便于单元测试注入内存 FS 做确定性测试;生产实现走 `std::fs`。目录遍历顺序:persona 的 `.md` 已按相对 posix 字符串排序;`_load_mcp_dir_entries`/`_load_mcps_from_dir` 只探测固定文件名,无顺序问题。

### C. 导入系统 / 注册表 / 运行时依赖 —— 无法逐行移植,需用 trait/注册表/配置表达

| 功能 | 位置 | 依赖 |
|---|---|---|
| `BuiltinToolSpec.build` | deep_agent_spec.py L297-328 | `_TOOL_PROVIDER_REGISTRY` + `ensure_builtin_elements_registered()`(导入系统) |
| `RailSpec.build` | deep_agent_spec.py L251-283 | `_RAIL_PROVIDER_REGISTRY` |
| `SubAgentSpec.build` | deep_agent_spec.py L355+ | 模型构造、注册表、运行时 |
| `resolve_plugin_parts` / `resolve_agent_template_parts` 中的物化循环 | resolver L175-186 | 依赖上述 build |
| `_render_params` 的 `workspace` 默认值 | resolver L257-263 | `ctx.workspace.root_path`(BuildContext 运行时载体) |
| `McpServerConfig.server_id` 缺省 | tool/mcp/base.py L45 | `uuid4().hex`(非确定性) |
| `LoadRecord.load_id` 缺省 | resolver L63 | `uuid4().hex`(非确定性) |
| `AgentCard.id` 缺省 | card.py L17 | `uuid4().hex`(非确定性) |
| `AgentCard.model_validate` | loader L222 | pydantic 校验(extra 忽略) |

> Rust 建议:`ToolProvider`/`RailProvider` trait + 注册表(与 Python 的 `_TOOL_PROVIDER_REGISTRY` 对齐);`BuildContext` 对应一个可序列化的上下文结构 + `workspace.root_path` 用显式字段;`uuid` 用 `uuid::Uuid::new_v4().simple()`。

### D. 非确定性/平台差异注意点(1:1 测试要小心)
1. **YAML 方言**:Python `yaml.safe_load`(YAML 1.1,`yes/no/on/off` → bool)vs Rust serde_yaml(YAML 1.2,这些是字符串)。影响 `enabled:` 字段(见 2.15 真值陷阱)。
2. **pydantic v2 校验语义**:`extra="forbid"` 拒绝未知键;`Literal` 严格匹配;str/int 字段的宽松强制(如 `priority: "10"` 字符串→int)需在 Rust 侧显式决定是否复刻。
3. **`resolve()` 语义**:Python 非 strict `resolve()` 不要求存在、解析符号链接并规范化;strict 版要求存在。Rust `canonicalize` 只做 strict 版。
4. **符号链接/大小写敏感**:`is_relative_to` 基于 resolve 后的规范化路径做前缀比较,天然防 `..` 逃逸;Windows 大小写敏感性按平台差异处理。
5. **文件编码**:所有读取固定 `utf-8`(读失败 → OSError/UnicodeDecodeError)。
6. **dict 键序**:Python dict 保持插入序;`_render_prompt_section` 的 content 键序、`_render_params` 的合并序都依赖此;Rust 侧用 `IndexMap`/`serde_json::Map`(preserve_order)复刻。

---

## 6. 功能点 → 行号索引(对照用)

### extension_loader.py(1005 行)
| 功能点 | 行号 |
|---|---|
| 常量区(8 组) | L26-101 |
| `_find_legacy_yaml_manifest` | L104-123 |
| `normalize_package_mcps`(公开) | L126-132 |
| `find_plugin_manifest`(公开) | L140-162 |
| `find_agent_template_manifest`(公开) | L165-180 |
| `load_plugin_package`(公开) | L183-193 |
| `load_agent_template_package`(公开) | L196-235 |
| `_load_plugin_manifest_json` | L238-263 |
| `_load_plugin_legacy_yaml` | L266-293 |
| `_load_agent_subtemplate` | L296-326 |
| `_first_display_description` | L329-339 |
| `_read_json_mapping` | L342-346 |
| `_resolve_new_manifest_path` | L349-370 |
| `_build_persona_prompt_sections` | L373-390 |
| `_build_model_spec` | L393-406 |
| `_build_tool_specs` | L409-424 |
| `_build_rail_specs` | L427-442 |
| `_build_skill_specs` + `_skill_mode` + `_skill_enabled_list` | L445-475 |
| `_build_mcp_specs` | L478-491 |
| `_load_mcp_dir_entries` | L494-502 |
| `_build_mcp_server_spec` | L505-524 |
| `_absolutize_legacy_resource` | L527-534 |
| `_absolutize_legacy_skill` | L537-541 |
| `_absolutize_legacy_mcp` | L544-551 |
| `_resolve_legacy_path` | L554-558 |
| `_normalize_legacy_plugin_yaml`(三分支) | L561-606 |
| `_remap_legacy_tools_to_rails` | L609-637 |
| `_drop_legacy_agent_control_fields` | L640-642 |
| `_merge_legacy_prompt_sections` | L645-657 |
| `_normalize_legacy_prompt_section` | L660-680 |
| `_normalize_legacy_section_content` | L683-690 |
| `_load_yaml_mapping` | L693-697 |
| `_load_data_file` | L700-704 |
| `_normalize_mcps` | L707-719 |
| `_looks_like_mcp_server_entry` | L722-734 |
| `_load_mcps_from_dir` | L737-761 |
| `_normalize_mcp_server_entry` | L764-809 |
| `_merge_sidecar_prompt_sections` | L812-835 |
| `_normalize_prompt_section` | L838-846 |
| `_resolve_section_file` | L849-856 |
| `_merge_list_file` | L859-865 |
| `_merge_items` | L868-872 |
| `_as_list` | L875-880 |
| `_section_content` | L883-884 |
| `_normalize_skill` / `_normalize_skills` | L887-900 |
| `_normalize_resource_items` | L903-907 |
| `_has_module_class_resource_spec` | L910-914 |
| `_normalize_resource_item` | L917-956 |
| `_normalize_builtin_resource_item` | L959-968 |
| `_normalize_entry_point_resource_item` | L971-976 |
| `_normalize_params` | L979-982 |
| `_pop_legacy_params` | L985-988 |
| `_module_to_package_file` | L991-996 |
| `__all__` | L999-1005 |

### extension_resolver.py(285 行)
| 功能点 | 行号 |
|---|---|
| `_TEMPLATE_PATTERN` | L38 |
| `ResourceKind` | L41-49 |
| `ResourceRef` | L52-57 |
| `LoadRecord` | L60-65 |
| `ResolvedSkill` | L68-74 |
| `ResolvedPromptSection` | L77-88 |
| `ExtensionParts` | L91-100 |
| `resolve_plugin_parts` | L103-116 |
| `resolve_agent_template_parts` | L119-150 |
| `_adapt_agent_template_to_subagent_spec` | L153-172 |
| `_resolve_tools` / `_resolve_rails` / `_as_built_list` | L175-194 |
| `_build_mcp_server_config` | L197-227 |
| `_resolve_prompt_section` / `_render_prompt_section_text` | L230-238 |
| `_resolve_skill` | L241-242 |
| `_select_content` | L245-254 |
| `_render_params` | L257-263 |
| `_render_template` | L266-273 |
| `__all__` | L276-285 |

### 其他文件
| 文件 | 功能点 | 行号 |
|---|---|---|
| `__init__.py` | re-export / `__all__` | L6-22 / L34-56 |
| `schema/extension_spec.py` | `_plain_data` / `_validate_plain_mapping` | L17-35 |
| 同上 | `_ExtensionSpecModel` | L38-41 |
| 同上 | `McpServerSpec` / `McpDirSpec` / `SkillSpec` / `PromptSectionSpec` | L44-95 |
| 同上 | `RubricSpec` / `MemorySpec` | L98-107 |
| 同上 | `AgentTemplateSpec` / `PluginSpec` | L110-169 |
| 同上 | `_require_absolute_path` / `validate_plugin_paths` | L172-204 |
| `builtin_rules.yaml` | 规则列表(10 条) | L15-84 |
| `schema/deep_agent_spec.py` | `TeamModelConfig`/`ModelSpec`、`RailSpec`、`BuiltinToolSpec`、`SubAgentSpec` | L116-133 / L245-283 / L286-328 / L336-353 |
| `schema/build_context.py` | `BuildContext` | L26-61 |
| `core/foundation/tool/mcp/base.py` | `McpServerConfig` | L44-51 |
| `core/single_agent/prompts/builder.py` | `PromptSection`、`SUPPORTED_LANGUAGES=("cn","en")`、`DEFAULT_LANGUAGE="cn"` | L9-18 / L21-43 |
| `core/single_agent/schema/agent_card.py` + `core/common/schema/card.py` | `AgentCard` / `BaseCard` | L16-28 / L9-19 |

---

## 7. 错误消息汇总(便于 1:1 断言)

> 均为 Python 字面消息;`{...}` 为 f-string 插值,`!r` 表示 `repr()`(字符串带引号)。Rust 移植建议把消息模板做成同字符串常量。

| # | 异常类型 | 消息模板 | 出处 |
|---|---|---|---|
| 1 | FileNotFoundError | `"{expected} not found: {requested_path}"`(expected = `"harness_config.yaml or expert_harness.yaml or harness.yaml"`) | loader L116-117, L121-122 |
| 2 | FileNotFoundError | `"Plugin manifest not found: {requested_path}"` | loader L160 |
| 3 | FileNotFoundError | `"manifest.json not found: {requested_path}"` | loader L175, L179 |
| 4 | ValueError | `"AgentTemplate manifest must be named 'manifest.json': {manifest}"` | loader L201 |
| 5 | ValueError | `"{manifest} declares packageType={payload.get('packageType')!r}, expected 'agent_template'"` | loader L205 |
| 6 | ValueError | `"AgentTemplate manifest {manifest} is missing required 'agentCard'"` | loader L209 |
| 7 | ValueError | `"{manifest} declares packageType={package_type!r}, expected 'plugin'"` | loader L242 |
| 8 | ValueError | `"Plugin manifest {manifest} must not declare {forbidden_key!r}"` | loader L245 |
| 9 | ValueError | `"Plugin manifest {manifest} is missing required 'id'"` | loader L249 |
| 10 | FileNotFoundError | `".subagent.json not found for subagent template: {subagent_dir}"` | loader L299 |
| 11 | ValueError | `"Subagent template {manifest} must not declare 'subagents' (only the root agent_template may declare direct subagents)"` | loader L302-305 |
| 12 | ValueError | `"Subagent template {manifest} is missing required 'agentName'"` | loader L308 |
| 13 | ValueError | `"Manifest must contain a mapping: {path}"` | loader L345 |
| 14 | ValueError | `"manifest path must be package-relative, got absolute path: {raw_path!r}"` | loader L362 |
| 15 | ValueError | `"manifest path {raw_path!r} escapes package root {package_root}"` | loader L365 |
| 16 | ValueError | `"manifest path {raw_path!r} must be a directory: {candidate}"` | loader L367 |
| 17 | ValueError | `"manifest path {raw_path!r} must be a file: {candidate}"` | loader L369 |
| 18 | ValueError | `"persona must be a mapping with 'dir': {persona!r}"` | loader L377 |
| 19 | ValueError | `"persona dir {persona_dir} has no markdown files"` | loader L387 |
| 20 | ValueError | `"model must be a mapping with 'file': {model_ref!r}"` | loader L397 |
| 21 | ValueError | `"model file {model_path} is missing the outer 'model' key"` | loader L405 |
| 22 | ValueError | `"tool entry must be a mapping with 'file': {item!r}"` | loader L413 |
| 23 | ValueError | `"rail entry must be a mapping with 'file': {item!r}"` | loader L431 |
| 24 | ValueError | `"skill entry must be a mapping with 'dir': {raw_item!r}"` | loader L450 |
| 25 | ValueError | `"skill mode must be 'all' or 'auto_list', got {value!r}"` | loader L467 |
| 26 | ValueError | `"enabled_skills must be a list, got {value!r}"` | loader L475 |
| 27 | ValueError | `"mcp entry must be a mapping: {item!r}"` | loader L482 |
| 28 | FileNotFoundError | `"MCP dir {mcp_dir} must contain one of: mcps.json, mcps.yaml"` | loader L502, L761 |
| 29 | ValueError | `"mcp entry is disabled and cannot be used: {item!r}"` | loader L508 |
| 30 | ValueError | `"mcp cwd must be package-relative, got absolute path: {raw_cwd!r}"` | loader L517 |
| 31 | ValueError | `"mcp cwd {raw_cwd!r} escapes package root {package_root}"` | loader L520 |
| 32 | ValueError | `"Harness manifest must contain a mapping: {path}"` | loader L696 |
| 33 | ValueError | `"Unsupported MCP transport/type: {transport!r}"` | loader L773 |
| 34 | OSError/FileNotFoundError | `resolve(strict=True)` / 读取失败的底层错误 | loader L190, L199, L363, L558 等 |
| 35 | ValueError | `"AgentTemplate subagent resolve requires BuildContext.extras['_parent_model']"` | resolver L130 |
| 36 | TypeError | `"SubAgentSpec.build returned {type(built).__name__}, expected SubAgentConfig"` | resolver L137 |
| 37 | ValueError | `"mapping keys must be strings"` | extension_spec L27 |
| 38 | ValueError | `"{type(value).__name__} is not JSON/YAML serializable data"` | extension_spec L30 |
| 39 | ValueError | `"{field_name} must already be an absolute path (the package loader or the caller of load_plugin_spec is responsible for absolutizing it): {value!r}"` | extension_spec L174-177 |
| 40 | ValueError(注册表) | `"Unknown tool type '{type}'. Registered types: [...]"` / `"Unknown rail type '{type}'. Registered types: [...]"` | deep_agent_spec L277-280 / L322-325 |

---

## 8. 移植建议顺序(供排期)

1. **P0(纯函数,先行)**:`_normalize_mcp_server_entry`、`_render_template`、`_select_content`、`_build_mcp_server_config`、`validate_plugin_paths`、`_normalize_resource_item` 系列、`_remap_legacy_tools_to_rails`、legacy prompt 归一化系列、全部数据模型(第 1 节)。
2. **P1(FS 层)**:`Fs` trait + `find_*_manifest`、`_resolve_new_manifest_path`、`_build_*_specs`、`_load_mcp_dir_entries`/`_load_mcps_from_dir`、legacy 绝对化、sidecar/list 文件合并。
3. **P2(运行时层)**:`ToolProvider`/`RailProvider` 注册表、`SubAgentSpec.build` 等价物、`BuildContext`、uuid 生成。
4. **P3(校验/测试)**:错误消息 1:1 断言(第 7 节表)、YAML 方言差异决策(第 5 节 D-1)、pydantic `extra="forbid"` 语义。
