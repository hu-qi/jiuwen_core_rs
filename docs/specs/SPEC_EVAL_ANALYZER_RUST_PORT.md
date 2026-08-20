# SPEC: evaluation_result_analyzer Rust 1:1 移植规格

> 源代码: `agent-core/openjiuwen/rsi/evaluation_result_analyzer/`(5 个文件:
> `interfaces.py`、`case_reader.py`、`signal_extractor.py`、`analyzer.py`、`__init__.py`)
> 目的: 为 Rust 1:1 移植提供逐函数、逐字段、逐规则的规格。所有行号均指 Python 源文件当前版本。
> 约定: 「确定性」= 纯函数/纯数据流,可 1:1 移植;「LLM 驱动」= 依赖外部模型服务,移植时保留调用契约与重试编排,提示词文本原样复制。

---

## 0. 模块总览与架构

### 0.1 包结构(`__init__.py`,共 35 行)

公开导出(`__all__`,行 24-35):

| 导出名 | 来源 | 说明 |
|---|---|---|
| `CaseAnalysisInput` | case_reader.py | 单 case 结构化输入 |
| `DeterministicSignals` | case_reader.py | 零-LLM 预提取信号 |
| `EvaluationResultAnalysisStrategy` | interfaces.py | 策略 Protocol |
| `EvaluationResultAnalyzer` | analyzer.py | 门面 |
| `EvaluationSummaryInput` | case_reader.py | 聚合摘要输入 |
| `SignalExtractor` | interfaces.py | 提取器 Protocol |
| `RewardSignalExtractor` | signal_extractor.py | reward 提取器(唯一被点名导出的具体提取器) |
| `build_analysis_strategy` | analyzer.py | 策略工厂 |
| `build_signal_extractor` | signal_extractor.py | 提取器工厂 |
| `register_signal_extractor` | signal_extractor.py | 注册表注册 |

### 0.2 架构(`analyzer.py` 头部 docstring,行 3-19)

```
DiagnosisAgentStrategy.analyze(invocation)
    → experience_learner.retrieve            (LLM/检索服务)
    → CaseReader (eval_ref / summary / case_inputs)   (确定性)
    → build_signal_extractor → SignalExtractor.extract  (确定性)
    → DeepAgent 两阶段 (per-case ‖ aggregation)        (LLM 驱动 + 确定性聚合)
    → EvaluationResultAnalysisArtifact

EvaluationResultAnalyzer (facade):
    mkdir → strategy.analyze → write issues.yaml + analysis_ref.yaml → return path
```

**关键架构事实**: 「两阶段」中的**第二阶段(aggregation)在当前代码里已经不再调用 LLM**——
`_aggregate_diagnosis`(analyzer.py 行 2548-2569)只调用确定性的 `_aggregate_structured_diagnoses`。
`AGGREGATION_SYSTEM_PROMPT`/`AGGREGATION_TEMPLATE`/`_build_aggregation_prompt`/`_compact_per_case_diagnoses`
仍是死代码(见 §6.9 与 §8.3)。Rust 移植可以保留或删除这些死代码,但**行为上不得让 aggregation 走 LLM**。

### 0.3 依赖的外部类型(不在本目录,移植需一并携带)

- `EvaluationResultAnalysisInvocation`(schema.py 行 194-204): `eval_ref_path: str`、`case_results_dir: str`、
  `case_traces_dir: str`、`team_skill_ref_path: str`、`harness_refs_path: str`、`output_dir: str`、
  `source_stage: str = ""`、`prior_candidate_feedback: dict = {}`。
- `TeamIssue`(schema.py 行 208-221): `issue_id`、`category`、`severity`、`summary`、`affected_cases: list[str]`、
  `evidence: list[dict]`、`suspected_team_scope`、`optimization_target`、`target_members: list[str]`、
  `recommendation`、`metadata: dict`。
- `EvaluationResultAnalysisArtifact`(schema.py 行 225-232): `analysis_id`、`analysis_ref_path`、
  `issues_path: str = ""`、`issues: list[TeamIssue]`、`metadata: dict`。
- `EvaluationResultAnalyzerConfig`(config/config.py 行 206-233):
  - `model_config_ref: str = ""`
  - `diagnosis_agent_model_config_ref: str = ""`
  - `diagnosis_agent_max_retries: int = DEFAULT_MODEL_CALL_MAX_RETRIES`
  - `diagnosis_agent_max_concurrency: int = 5`(**注意:代码中未被使用**,见 §6.6)
  - `diagnosis_agent_max_iterations: int = 20`
  - `max_issues: int = 20`
  - `evidence_limit_per_issue: int = 5`
  - `output_filename: str = "issues.yaml"`
  - 提供 `from_dict`(行 218-233),`_int_value` 取整、`_bool_value` 取布尔。

### 0.4 两个 Protocol(`interfaces.py`,共 63 行)

- `SignalExtractor`(行 21-35): `@property name -> str`(行 24-27,稳定提取器名,用于日志与元数据);
  `extract(summary: EvaluationSummaryInput, case_inputs: list[CaseAnalysisInput]) -> DeterministicSignals`(行 29-35)。
  语义: **零-LLM** 预提取,不发起任何模型调用。Protocol 即 duck-typing 契约;Rust 用 `trait` 表达。
- `EvaluationResultAnalysisStrategy`(行 38-57): `@property name -> str`(行 47-50);
  `async analyze(invocation: EvaluationResultAnalysisInvocation) -> EvaluationResultAnalysisArtifact`(行 52-57)。
  语义: 实现只接收原始 invocation,自行负责数据加载、信号提取、推理。
- `__all__` 行 60-62。

---

## 1. case_reader.py —— 文件系统读取层(全部确定性)

### 1.1 `EvaluationSummaryInput`(行 15-23,`frozen=True, slots=True` dataclass)

| 字段 | 类型 | 默认值 |
|---|---|---|
| `total_cases` | int | 0 |
| `passed_count` | int | 0 |
| `failed_count` | int | 0 |
| `average_score` | float | 0.0 |
| `evaluation_method` | str | "" |

### 1.2 `DeterministicSignals`(行 26-34,`frozen=True, slots=True`)

| 字段 | 类型 | 默认值 |
|---|---|---|
| `method` | str | "" |
| `exec_failures` | list[str] | `[]` |
| `judge_failures` | list[str] | `[]` |
| `error_clusters` | list[dict[str, Any]] | `[]` |
| `method_specific` | dict[str, Any] | `{}` |

注意: `expected_mismatch_cases` / `missing_reference_cases` **不是**本结构的字段,而是由
GenericSignalExtractor 塞进 `method_specific` 的键。

### 1.3 `CaseAnalysisInput`(行 37-57,`frozen=True, slots=True`)

| 字段 | 类型 | 默认值 | 说明 |
|---|---|---|---|
| `case_id` | str | (必填) | |
| `status` | str | (必填) | 如 `"failed"` |
| `score` | float | (必填) | |
| `input` | str | (必填) | 来自 trace.json |
| `expected` | str \| None | (必填) | **CaseReader 恒置 None**(行 157) |
| `response` | str | (必填) | |
| `error` | str | (必填) | |
| `evaluation_method` | str | (必填) | |
| `evaluation_passed` | bool | (必填) | |
| `evaluation_reason` | str | (必填) | |
| `evaluation_metadata` | dict[str, Any] | (必填) | |
| `trace_path` | str | (必填) | |
| `result_path` | str | (必填) | |
| `trajectory_window_summary` | dict[str, Any] | `{}` | 有界轨迹窗口摘要 |
| `normalized_trace_summary` | dict[str, Any] | `{}` | 有界归一化轨迹摘要 |
| `training_signal` | dict[str, Any] | `{}` | |
| `benchmark_test_contract` | dict[str, Any] | `{}` | |

### 1.4 `CaseReader`(行 60-176)

#### `read_eval_ref(path: str) -> dict`(静态方法,行 63-79)
- `p = Path(path)`;若 `not p.exists()` → **raise `ValueError(f"eval_ref path not found: {path}")`**(行 77-78)。
- 否则 `yaml.safe_load(p.read_text(encoding="utf-8")) or {}`(行 79):YAML 解析结果 falsy 时返回空 dict。

#### `read_summary(path: str) -> EvaluationSummaryInput`(静态方法,行 81-107)
- `path` 为空串 → 返回全默认 `EvaluationSummaryInput()`(行 95-96)。
- 文件不存在 → 同样返回全默认(行 97-99)。**不抛错**。
- 否则 `json.loads`(行 100;**无 try/except**——JSON 损坏时向上抛 `JSONDecodeError`,Rust 移植应等价报错)。
- G4 字段映射(行 101-107):
  - `total_cases = int(data.get("total_cases", 0))`
  - `passed_count = int(data.get("passed_cases", data.get("passed_count", 0)))` —— **优先 `passed_cases`,回退 `passed_count`,再回退 0**
  - `failed_count = int(data.get("failed_cases", data.get("failed_count", 0)))` —— 同理
  - `average_score = float(data.get("average_score", 0.0))`
  - `evaluation_method = str(data.get("evaluation_method", ""))`
- 注意 `int()`/`float()` 是强制转换:字符串 `"3"` 可转,非数值字符串会抛 `ValueError`(移植时保持一致或文档化差异)。

#### `read_case_inputs(directory: str) -> list[CaseAnalysisInput]`(静态方法,行 109-176)
- `directory` 不是目录 → 返回 `[]`(行 123-125)。
- `dataset_case_cache: dict[Path, dict[str, dict]]` 跨 case 缓存数据集解析结果(行 128)。
- 遍历 `sorted(case_results_dir.glob("*/result.json"))`(行 129,按路径字典序;**glob 不含隐藏目录**)：
  - `case_dir = result_path.parent`;`trace_path = case_dir / "trace.json"`(行 130-131)。
  - `result_data = json.loads(result_path.read_text(...))`(行 133,无异常保护)。
  - `trace_data`:若 trace.json 存在则加载,否则 `{}`(行 134-136)。
  - `evaluation = result_data.get("evaluation") or {}`;`eval_metadata = evaluation.get("metadata") or {}`(行 138-139)。
  - `result_metadata = result_data.get("metadata") or {}`;`training_signal = result_metadata.get("training_signal") if isinstance(result_metadata, dict) else {}`;
    若非 dict 再置 `{}`(行 140-143)。
  - `case_id = str(result_data.get("case_id", case_dir.name))`(行 144)。
  - `benchmark_test_contract = _read_benchmark_test_contract(case_id=..., result_metadata=..., dataset_case_cache=...)`(行 145-149)。
  - 组装 `CaseAnalysisInput`(行 151-173),各字段映射规则:
    - `status = str(result_data.get("status", ""))`
    - `score = float(result_data.get("score") or 0.0)` —— **falsy(含 None/0/"")→ 0.0**
    - `input = str(trace_data.get("input", ""))`
    - `expected = None`(**恒为 None**)
    - `response = str(trace_data.get("response", result_data.get("result", "")))` —— **优先 trace.response,回退 result.json 的 `result` 键,再回退 ""**
    - `error = str(result_data.get("error") or "")`
    - `evaluation_method = str(evaluation.get("method", ""))`
    - `evaluation_passed = bool(evaluation.get("passed", False))` —— 注意 Python `bool()`:字符串 `"false"` 也为 True
    - `evaluation_reason = str(evaluation.get("reason", ""))`
    - `evaluation_metadata = eval_metadata`(原样引用)
    - `trace_path / result_path = str(路径)`
    - `trajectory_window_summary = _bounded_trajectory_window_summary(trace_data.get("behavior_trace", {}))`
    - `normalized_trace_summary = _bounded_normalized_trace_summary(trace_data.get("behavior_trace", {}), case_dir=case_dir)`
    - `training_signal`、`benchmark_test_contract` 原样。
- 返回按 case 目录名排序的列表(由 sorted glob 保证)。

### 1.5 私有辅助函数

#### `_read_benchmark_test_contract(*, case_id, result_metadata, dataset_case_cache) -> dict`(行 179-221)
- `result_metadata` 非 dict → `{}`(行 186-187)。
- `raw_case_path = result_metadata.get("case_path")`;非 str 或 strip 后为空 → `{}`(行 188-190)。
- `dataset_path = Path(raw_case_path).expanduser()`;相对路径 → `Path.cwd() / dataset_path`;再 `.resolve()`(行 192-195)。
- 缓存: `dataset_case_cache.get(dataset_path)` 未命中则 `_read_dataset_cases(dataset_path)` 并写入(行 197-200)。
- `case_data = cases_by_id.get(case_id)`;非 dict → `{}`(行 202-204)。
- `contract = case_data.get("verification_contract")`;非 dict → `{}`(行 205-207)。
- `fail_to_pass = _test_id_list(contract.get("fail_to_pass", contract.get("must_pass")))`(行 209)
- `pass_to_pass = _test_id_list(contract.get("pass_to_pass", contract.get("regression")))`(行 210)
- `test_patch = contract.get("test_patch", contract.get("acceptance_probe"))`;非 str → `""`(行 211-212)。
- 三者(两组列表 + test_patch)全空 → `{}`(行 213-214)。
- 返回(行 216-221):
  ```json
  {
    "provenance": "<dataset_path>#case_id=<case_id>.verification_contract",
    "fail_to_pass": [...], "pass_to_pass": [...], "test_patch": "..."
  }
  ```

#### `_read_dataset_cases(dataset_path: Path) -> dict[str, dict]`(行 224-239)
- `json.loads`;`OSError` / `JSONDecodeError` → `{}`(行 225-228)。
- `raw_cases = dataset.get("cases") if isinstance(dataset, dict) else dataset`;非 list → `{}`(行 229-231)。
- 每个 case 非 dict 跳过;`case_id = str(case.get("case_id", case.get("instance_id", ""))).strip()`;非空才登记 `cases[case_id] = case`(行 233-238)。**后出现覆盖先出现**。

#### `_test_id_list(value: Any) -> list[str]`(行 242-251)
- str → 尝试 `json.loads`;解析成功且为 list → 用之;否则 `[value]`(行 243-248)。
- 非 list → `[]`(行 249-250)。
- `[str(item) for item in value if str(item).strip()]`(行 251):**过滤空白项**。

#### `_bounded_trajectory_window_summary(behavior_trace) -> dict`(行 254-277)
- 非 dict → `{}`;`summary = behavior_trace.get("trajectory_window_summary")` 非 dict → `{}`(行 255-260)。
- 基础(行 262-266): `window_size`、`event_count` 原值;`failure_signatures = _string_list(summary.get("failure_signatures"), 20)`。
- `recent_events` 为 list 时(行 267-276): 取前 **30** 个 dict 项,每项:
  `{"event_index": item.get("event_index"), "event_type": _excerpt(str(item.get("event_type","")), 80), "summary": _excerpt(str(item.get("summary","")), 1200)}`。

#### `_bounded_normalized_trace_summary(behavior_trace, *, case_dir) -> dict`(行 280-325)
- `trace_path = _resolve_normalized_trace_path(behavior_trace, case_dir=case_dir)`;非文件 → `{}`(行 282-284)。
- 读 JSON;`OSError`/`JSONDecodeError` → `{}`(行 285-288)。
- `traces = data.get("traces")`;list 时遍历**前 4** 个 trace(行 290-292):
  - 每个 trace 的 `messages` list 遍历**前 40** 条 message(行 298-300),每条:
    `{"role": _excerpt(...,80), "message_index": message.get("message_index"), "content": _excerpt(...,1200), "step_pointer": _excerpt(...,120), "tool_calls": _bounded_tool_calls(message.get("tool_calls"))}`(行 302-310)。
  - trace 级(行 311-321): `trace_id`(_excerpt 240)、`member_id`(160)、`member_role`(160)、`execution_id`(160)、
    `step_count`、`message_count` 原值、`messages`。
- 返回 `{"case_id": _excerpt(str(data.get("case_id","")), 160), "traces": bounded_traces}`(行 322-325)。

#### `_resolve_normalized_trace_path(behavior_trace, *, case_dir) -> Path | None`(行 328-335)
- `behavior_trace` 为 dict 且 `normalized_trace_path` 为非空 str: 绝对路径原样,相对路径 `case_dir / path`(行 329-333)。
- 否则默认 `case_dir / "judge" / "normalized_trace.json"`(行 334-335)。**永不返回 None**(签名里的 `| None` 是历史的)。

#### `_bounded_tool_calls(value) -> list[dict]`(行 338-354)
- 非 list → `[]`;取**前 8** 个 dict 项,每项:
  `{"name": _excerpt(str(item.get("name","")),120), "input": _excerpt(str(item.get("input","")),1200), "output": _excerpt(...,1200), "error": _excerpt(...,1200), "step_pointer": _excerpt(...,120)}`。

#### `_string_list(value, limit) -> list[str]`(行 357-360)
- 非 list → `[]`;`[str(item) for item in value[:limit]]`(**不做空白过滤**)。

#### `_excerpt(text, max_chars) -> str`(行 363-366)
- `len(text) <= max_chars` → 原样;否则 `text[:max_chars - 3] + "..."`(注意:**无 Unicode 感知**,按字符数切片,Python 中即按码点)。

---

## 2. signal_extractor.py —— 确定性信号提取(全部确定性)

### 2.1 错误指纹 `_fingerprint_error`(行 31-38)

5 个正则(行 24-28,均为模块级编译):

| 模式名 | 正则(Python 原始串) | 替换串 | 行号 |
|---|---|---|---|
| `_TIMESTAMP_PATTERN` | `\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})?` | `<ts>` | 27 |
| `_UUID_PATTERN` | `\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b` | `<uuid>` | 28 |
| `_PATH_PATTERN` | `[A-Za-z]:[/\\][^\s,;'"\]]+|/[/\w.\-]+` | `<path>` | 24 |
| `_HEX_PATTERN` | `\b[0-9a-fA-F]{6,}\b` | `<hex>` | 25 |
| `_LINE_PATTERN` | `:\d+` | `:<N>` | 26 |

**替换顺序(严格)**: 时间戳 → UUID → 路径 → hex → 行号(行 33-37)。
每步 `re.sub` 作用于**上一步之后的完整文本**,非重叠、最左匹配(标准 Python `re.sub` 语义;Rust 用 `regex` crate 可等价)。

细节与陷阱(Rust 移植必须逐条保持):
1. **顺序的意义**: 时间戳先于行号替换,避免 `2024-01-01T00:00:00` 中的 `:00` 被 `:\d+` 吃掉;
   UUID 先于 hex,避免 UUID 片段被 `\b[0-9a-fA-F]{6,}\b` 命中。
2. `_PATH_PATTERN` 两个分支: ① Windows 盘符路径 `[A-Za-z]:[/\\]` 后跟任意非空白、非 `,;'"\]` 字符;
   ② POSIX 风格 `/[/\w.\-]+` —— 至少一个 `/`(或单词字符/`.`/`-`)跟在 `/` 后;**单独的 `/` 不匹配**。
   路径不含 `:`(`:` 不在字符类),因此 `file.py:123` 中路径止于冒号,行号留给 `_LINE_PATTERN`。
3. `_HEX_PATTERN` 有词边界 `\b`,匹配 6 个及以上连续十六进制字符。
4. `_LINE_PATTERN` 替换 `:123` 为 `:<N>`(**保留冒号**);无冒号的裸数字不动。
5. **最终空白归一**(行 38): `" ".join(text.split())` —— 按任意空白切分后以单空格连接,并去掉首尾空白。
   Rust 等价: `text.split_whitespace().collect::<Vec<_>>().join(" ")`。

### 2.2 `GenericSignalExtractor`(行 46-103,默认 / exact_match)

- `name: str = "generic"`(行 49)。
- `extract(summary, case_inputs) -> DeterministicSignals`(静态方法,行 51-103):
  1. `method = summary.evaluation_method`(行 66)。
  2. 初始化 `exec_failures`、`judge_failures`、`expected_mismatch_cases`、`missing_reference_cases`、`error_map: dict[str, list[str]]`(行 68-72)。
  3. 逐 case(行 74-90):
     - `if case.status == "failed":` → `exec_failures.append(case_id)`;`elif not case.evaluation_passed:` → `judge_failures.append(case_id)`(行 75-78)。**互斥**: 一个 case 只会进其中一个。
     - `if case.error:`(truthy 字符串)→ `fp = _fingerprint_error(case.error)`;`error_map.setdefault(fp, []).append(case_id)`(行 80-82)。
     - `if case.status != "failed":`(行 86)—— **只对执行成功的 case** 检查 expected/response:
       - `case.expected is None` → `missing_reference_cases`(行 87-88)
       - `elif case.expected != case.response` → `expected_mismatch_cases`(行 89-90)
  4. `error_clusters = [{"fingerprint": fp, "cases": cases} for fp, cases in error_map.items()]`(行 92)—— **保持首次出现顺序**(Python dict 插入序)。
  5. 返回(行 94-103): `method_specific = {"expected_mismatch_cases": ..., "missing_reference_cases": ...}`。

### 2.3 `PytestSignalExtractor`(行 111-180,script_based)

- `name: str = "pytest"`(行 119)。
- `extract`(静态方法,行 121-180):
  1. `generic = GenericSignalExtractor().extract(summary, case_inputs)`(行 137,新建实例)。
  2. 逐 case(行 143-156): `evidence = case.evaluation_metadata.get("evidence")`;
     - falsy → `evidence_missing.append(case_id)` 并 `continue`(行 145-147)。
     - 若 `evidence` 是 list,逐项 dict: `test_name = str(item.get("test_name", "unknown"))` →
       `failed_test_clusters[test_name].append(case_id)`;`a_type = str(item.get("assertion_type", "unknown"))` →
       `assertion_type_clusters[a_type].append(case_id)`(行 150-156)。
  3. **回退触发条件**: `if evidence_missing:`(只要有任一 case 缺 evidence,行 158)—— 返回(行 159-169):
     ```json
     method_specific = {
       ...generic.method_specific,
       "evidence_missing_cases": evidence_missing,
       "fallback_reason": "pytest_evidence_missing"
     }
     ```
     已收集的 `failed_test_clusters`/`assertion_type_clusters` **被丢弃**。
  4. 否则(行 171-180): `method_specific = {"failed_test_clusters": ..., "assertion_type_clusters": ...}`。
  5. 两个分支都携带 `generic` 的 exec/judge_failures/error_clusters;`method = summary.evaluation_method`。

### 2.4 `RewardSignalExtractor`(行 188-250,reward_based)

- `name: str = "reward"`(行 191)。
- `extract`(静态方法,行 193-250):
  1. `generic` 先行(行 200)。
  2. 逐 case(行 209-234):
     - `reward = case.evaluation_metadata.get("reward")`;`if reward == 0 or reward == 0.0:` → `zero_reward_cases.append(case_id)`(行 210-212)。注意 Python 相等语义: `None == 0` 为 False;字符串 `"0"` 为 False。
     - `response_text = str(case.response)`;`if "Max iterations reached without completion" in response_text:` → `max_iteration_cases.append(case_id)`(**子串匹配,区分大小写**,行 214-216)。
     - `if case.case_id in zero_reward_cases or case.case_id in max_iteration_cases:` → `member_harness_issue_cases.append(case_id)`(行 218-219;此处列表只含当前 case 自身刚追加的 id)。
     - `window = case.trajectory_window_summary`;dict 时(行 221-230):
       - `signatures = _string_list(window.get("failure_signatures"), 20)`;非空 → 记入 `trajectory_failure_signatures_by_case[case_id]`,
         且**若 case_id 尚不在 member_harness_issue_cases 则追加**(行 223-227)。
       - `recent_events = _bounded_recent_events(window.get("recent_events"))`;非空 → 记入 `trajectory_recent_events_by_case`(行 228-230)。
     - `attribution = _normalized_trace_attribution(case)`;非空 → 记入 `normalized_trace_attribution_by_case`(行 232-234)。
  3. 返回(行 236-250): `method_specific = {**generic.method_specific, "zero_reward_cases", "max_iteration_cases", "member_harness_issue_cases", "trajectory_failure_signatures_by_case", "trajectory_recent_events_by_case", "normalized_trace_attribution_by_case"}`。

### 2.5 `AtomicChecksSignalExtractor`(行 258-336,atomic_checks)

- `name: str = "atomic_checks"`(行 261)。
- `extract`(静态方法,行 263-336):
  1. `generic` 先行(行 269)。
  2. 逐 case(行 278-320):
     - `raw_checks = case.evaluation_metadata.get("atomic_checks", [])`;list 时逐项(行 283-299):
       - `name = str(check.get("name", "") or "").strip()`;空名 → `continue`(行 287-289)。
       - `check.get("passed") is True`(**严格 is True**,布尔身份)→ `passed_checks.append(name)`(行 290-291)。
       - `elif str(check.get("status", "")) != "skipped":` → `failed_checks.append(name)`;
         `failed_details.append({"name": name, "detail": str(check.get("detail", "") or "")[:2000]})`(行 292-299;**detail 截断 2000 字符,无省略号**)。
     - `atomic_checks_by_case[case_id] = {"passed": passed_checks, "failed": failed_checks}`(**恒写入**,含空列表,行 300-303)。
     - `if failed_details:` → 记入 `failed_check_details_by_case`,并 `member_harness_issue_cases.append(case_id)`(行 304-306)。
     - `if 0.0 < case.score < 1.0:` → `partial_score_cases.append(case_id)`(行 307-308)。
     - window 处理与 reward 相同(行 310-317),但**签名存在时不会**追加 `member_harness_issue_cases`(与 reward 不同)。
     - `_normalized_trace_attribution` 同上(行 318-320)。
  3. **exec/judge 判定与 generic 不同**(行 324-325,注意):
     - `exec_failures = [case.case_id for case in case_inputs if case.error]` —— 按 **error 非空** 而非 status=="failed"。
     - `judge_failures = [case.case_id for case in case_inputs if not case.error and not case.evaluation_passed]`。
     - `error_clusters = generic.error_clusters`(行 326)。
  4. `method_specific`(行 327-335): `{"atomic_checks_by_case", "failed_check_details_by_case", "partial_score_cases", "member_harness_issue_cases", "trajectory_failure_signatures_by_case", "trajectory_recent_events_by_case", "normalized_trace_attribution_by_case"}`。
     **注意:不合并 `generic.method_specific`**(与 Pytest/Reward/LlmJudge 不同)—— expected/missing_reference 信号在 atomic_checks 下丢失,属有意行为,移植不得"修正"。

### 2.6 `LlmJudgeSignalExtractor`(行 344-426,llm_as_judge)

- `name: str = "llm_judge"`(行 352)。
- `extract`(静态方法,行 354-426):
  1. `generic` 先行(行 370)。
  2. 逐 case(行 379-395):
     - `parsed = case.evaluation_metadata.get("parsed", {})`;`dimensions = parsed.get("dimensions") if isinstance(parsed, dict) else None`(行 380-381)。
     - `if not dimensions or not isinstance(dimensions, dict):` → `rationale_missing.append(case_id)`;`continue`(行 383-385)。
     - `low_score_behaviors[cid] = dimensions.get("low_score_behaviors") or []`(行 388,**无类型检查**)
     - `behavior_score_distribution[cid] = dimensions.get("per_behavior_scores") or {}`(行 389)
     - `behavior_diagnostics[cid] = _behavior_diagnostics(dimensions.get("behavior_diagnostics"))`(行 390)
     - `avg_behavior_score[cid] = float(dimensions.get("avg_behavior_score", 0.0))`(行 391)—— 键缺失→0.0;**键存在但为 None → `float(None)` 抛 TypeError;非数值串 → ValueError**(潜在崩溃点,移植须保持一致或显式处理)。
     - `behavior_pass_fail_counts[cid] = {"pass_count": int(dimensions.get("pass_count", 0)), "fail_count": int(dimensions.get("fail_count", 0))}`(行 392-395,同上 int() 陷阱)。
  3. **全量回退**: `if rationale_missing and len(rationale_missing) == len(case_inputs):`(行 397,注意是**全部** case 缺失才回退,与 Pytest 的"任一缺失"不同)→ 返回(行 398-408):
     ```json
     method_specific = {
       ...generic.method_specific,
       "rationale_missing_cases": rationale_missing,
       "fallback_reason": "parsed_dimensions_missing"
     }
     ```
  4. 否则(行 410-426): `method_specific = {"low_score_behaviors", "behavior_score_distribution", "behavior_diagnostics", "avg_behavior_score", "behavior_pass_fail_counts"}`;
     若 `rationale_missing` 非空(**部分缺失**)→ 追加 `"rationale_missing_cases": rationale_missing`(行 417-418)。

### 2.7 注册表(行 433-477)

- `_EXTRACTOR_MAP: dict[str, extractor实例]`(行 433-445,模块级、进程内可变全局):
  - `"script_based" → PytestSignalExtractor()`
  - `"reward_based" → RewardSignalExtractor()`
  - `"atomic_checks" → AtomicChecksSignalExtractor()`
  - `"llm_as_judge" → LlmJudgeSignalExtractor()`
  - 注意: 内置实例在 import 时创建;**GenericSignalExtractor 不在表中**。
- `register_signal_extractor(method, extractor) -> None`(行 448-455):
  - `normalized = str(method).strip()`;空 → `raise ValueError("signal extractor method must be non-empty")`(行 450-452)。
  - `if not callable(getattr(extractor, "extract", None)):` → `raise TypeError("signal extractor must define extract(summary, case_inputs)")`(行 453-454)。
  - `_EXTRACTOR_MAP[normalized] = extractor`(行 455)—— **覆盖内置条目**,全局生效。
- `build_signal_extractor(method) -> extractor`(行 458-477):
  - `return _EXTRACTOR_MAP.get(method, GenericSignalExtractor())`(行 477)。
  - **不做 strip**;未注册方法(含 `"default"`、`"exact_match"`、空串)→ **每次新建** `GenericSignalExtractor()`(非单例)。
  - 已注册方法返回注册实例(共享同一对象;Pytest/Reward/Atomic/LlmJudge 实例无内部状态,共享安全)。

### 2.8 信号层辅助函数

#### `_string_list(value, limit)`(行 480-483)
- 非 list → `[]`;`[str(item) for item in value[:limit]]`。与 case_reader 版本(行 357-360)语义一致。

#### `_bounded_recent_events(value) -> list[dict[str, str]]`(行 486-499)
- 非 list → `[]`;取**前 20** 个 dict 项,每项:
  `{"event_type": str(item.get("event_type", ""))[:80], "summary": str(item.get("summary", ""))[:1200]}`。
- 与 case_reader 的 `_bounded_trajectory_window_summary` 不同: 这里**按码点硬截断、无省略号、无 event_index**。

#### `_normalized_trace_role(trace: dict) -> str`(行 558-562)
- `role = str(trace.get("member_role") or trace.get("member_id") or "").strip()`。
- `role in {"", "team", "unattributed"}` → 返回 `""`;否则原样返回。

#### `_normalized_trace_attribution(case) -> dict`(行 502-555)
- `trace_summary = case.normalized_trace_summary`;非 dict → `{}`(行 504-506)。
- 双层遍历: 每个 trace(dict 检查)→ 每个 message(dict 检查)→ 每个 tool_call(dict 检查)(行 507-524)。
  - `message_index = message.get("message_index")`(原值,可为 None)。
  - `error = str(call.get("error", "") or "")`;**空则跳过该 call**(行 525-527)。
- **命中第一个带 error 的 tool_call 即返回**(行 532-554,最早失败步骤优先):
  - `tool_name = str(call.get("name", "tool"))`
  - `step_pointer = str(call.get("step_pointer", "") or "")`
  - `command = str(call.get("input", "") or "")`
  - `display_role = role or "unattributed execution"`
  - `evidence_ref = {"trace_id": trace_id, "role": role, "message_index": message_index, "step_pointer": step_pointer}`(注意 role 用**未修饰**值,可为 "")
  - 返回:
    ```json
    {
      "root_cause": "<display_role> encountered a failing <tool_name> step and did not recover before the verifier failed.",
      "critical_mistake": "Earliest decisive failed step: <step_pointer or 'unknown step'>; <tool_name> input=<command[:240] repr>; error=<error[:240] repr>.",
      "general_mechanism": "Strengthen the member workflow so failed commands or verifier errors trigger inspection, targeted correction, and re-verification before completion.",
      "target_ref": "member_harness.<role>.skill" if role else "unassigned",
      "evidence_refs": [evidence_ref],
      "confidence": "medium" if role else "low"
    }
    ```
- **关键移植点**: `critical_mistake` 使用 Python `f"...{command[:240]!r}..."` —— `!r` 是 **repr()**,即带单引号、对 `\`、`'`、控制字符转义的字面量。Rust 端需实现等价 repr(可用 `format!("{:?}", s)` 近似,但注意 Rust `{:?}` 用双引号、转义规则与 Python repr 有差异;若要逐字节一致需手写 Python-repr)。
- 无任何命中 → `{}`。

#### `_behavior_diagnostics(value) -> dict[str, dict[str, str]]`(行 565-579)
- 非 dict → `{}`;逐 `behavior_id -> raw`:`raw` 非 dict 跳过;`diagnostics[str(behavior_id)]` =
  `{"reason": str(raw.get("reason","") or ""), "failure_reason": ..., "missing_capability": ..., "suggested_surface_hint": ..., "evidence": ...}`(均 `or ""` 归一)。

---

## 3. analyzer.py —— 主流程(混合:确定性 + LLM)

### 3.0 常量(行 77-85)

| 常量 | 值 | 用途 |
|---|---|---|
| `_TEXT_SNIPPET_CHARS` | 1200 | 文本片段截断 |
| `_METADATA_SNIPPET_CHARS` | 2000 | 元数据截断 |
| `_EXPERIENCE_SNIPPET_CHARS` | 2000 | 经验截断 |
| `_CANDIDATE_FEEDBACK_CHARS` | 8000 | 候选反馈截断 |
| `_SIGNAL_SNIPPET_CHARS` | 2500 | 信号截断 |
| `_EVIDENCE_SUMMARY_CHARS` | 6000 | evidence summary 截断 |
| `_AGGREGATION_DIAGNOSIS_CHARS` | 1200 | 聚合诊断截断 |
| `_AGGREGATION_SIGNAL_CHARS` | 4000 | 聚合信号截断 |
| `_RAW_OUTPUT_CHARS` | 512 | 原始输出截断 |

### 3.1 提示词常量

- `DIAGNOSIS_SYSTEM_PROMPT`(行 92-443): 逐 case 诊断系统提示词(反向归因方法论、Target Reference 语义、
  per-case JSON schema、反模糊规则)。**移植时原样复制**,内含两个 JSON schema(per-case 行 337-377、aggregation 行 379-426)。
- `PER_CASE_DIAGNOSIS_TEMPLATE`(行 445-458): 占位符 `{stage_instruction}`、`{evidence_instruction}`、`{diagnosis_input}`。
- `AGGREGATION_SYSTEM_PROMPT`(行 460-544): 聚合系统提示词(**当前无调用点**,死代码)。
- `AGGREGATION_TEMPLATE`(行 546-579): 占位符 `{total_cases}`、`{passed_count}`、`{failed_count}`、`{average_score}`、
  `{evaluation_method}`、`{per_case_diagnoses}`、`{retrieved_experience}`、`{exec_failures}`、`{judge_failures}`、
  `{error_clusters}`、`{method_specific}`、`{max_issues}`、`{evidence_limit_per_issue}`、`{stage_instruction}`(**当前无调用点**,死代码)。

### 3.2 主流程 `DiagnosisAgentStrategy`(行 2251-2587)

- `name: str = "diagnosis_agent"`(行 2265)。
- `__init__(config: EvaluationResultAnalyzerConfig, experience_learner: OptimizationExperienceLearner | None = None)`(行 2267-2276):
  - `self._config = config`;`self._experience_learner = experience_learner or OptimizationExperienceLearner(OptimizationExperienceLearnerConfig())`;
    `self._case_reader = CaseReader()`。

#### `async analyze(invocation) -> EvaluationResultAnalysisArtifact`(行 2278-2366)

步骤(逐字顺序):

1. `retrieved_experience = await self._retrieve_experience(invocation)`(行 2297,LLM/检索服务)。
2. `eval_ref = self._case_reader.read_eval_ref(invocation.eval_ref_path)`(行 2299,确定性)。
3. `summary = self._case_reader.read_summary(eval_ref.get("summary_path", ""))`(行 2300,确定性)。
4. `case_inputs = self._case_reader.read_case_inputs(invocation.case_results_dir)`(行 2301,确定性)。
5. **早退**: `if not case_inputs:` → 返回 artifact(行 2303-2313):
   ```json
   { "analysis_id": Path(output_dir).name, "analysis_ref_path": "", "issues": [],
     "metadata": {"analysis_status": "empty_case_results", "model_config_ref": config.model_config_ref, "retrieved_experience": ...} }
   ```
6. `eval_method = summary.evaluation_method or "default"`;**空方法名归一为 "default"**(行 2315)。
7. `extractor = build_signal_extractor(eval_method)`;`signals = extractor.extract(summary, case_inputs)`(行 2316-2317,确定性)。
8. `model_config_ref = config.diagnosis_agent_model_config_ref or config.model_config_ref`;为空 → `_partial_artifact(invocation, "model_config_ref must be set", retrieved_experience)`(行 2319-2321)。
9. `output_dir = Path(invocation.output_dir).expanduser().resolve()`;`mkdir(parents=True, exist_ok=True)`(行 2323-2324)。
10. `diagnosis_case_inputs = [case for case in case_inputs if not case.evaluation_passed]`(行 2326)—— **只诊断未通过 case**。
11. `per_case_results = await self._per_case_diagnosis(diagnosis_case_inputs, signals, retrieved_experience, source_stage=invocation.source_stage, prior_candidate_feedback=invocation.prior_candidate_feedback)`(行 2327-2333,LLM 驱动)。
12. 写 `output_dir / "per_case_diagnoses.json"`: `{"per_case_diagnoses": per_case_results}`(行 2334-2335)。
13. `diagnosis_failed_count = sum(1 for item in per_case_results if item.get("analysis_failed"))`(行 2336)。
14. `issues = await self._aggregate_diagnosis(per_case_results, summary, signals, retrieved_experience, output_dir=..., source_stage=...)`(行 2338-2345,**实际是确定性分组**,见 §3.5)。
15. `issues = [_apply_g5_mapping(issue) for issue in issues]`;`issues = issues[:config.max_issues]`(行 2348-2349)。
16. 返回 artifact(行 2351-2366),metadata:
    ```json
    { "analysis_status": "completed", "strategy": "diagnosis_agent",
      "model_config_ref": diagnosis_agent_model_config_ref or model_config_ref,
      "signals_method": signals.method, "per_case_count": len(case_inputs),
      "diagnosed_case_count": len(diagnosis_case_inputs),
      "diagnosis_failed_count": diagnosis_failed_count,
      "per_case_diagnoses_path": str(per_case_diagnoses_path),
      "retrieved_experience": retrieved_experience }
    ```

#### `_retrieve_experience(invocation) -> dict`(行 2368-2380,LLM/检索服务边界)
- 调用 `experience_learner.retrieve_member_stage_experience(stage="evaluation_result_analysis", eval_ref_path=..., analysis_result_path="", harness_refs_path=..., candidate_modules=["team_skill", "member_harness"])`。
- 返回 `{"stage": retrieval.query.stage, "matches": retrieval.matches, "metadata": retrieval.metadata}`。
- **移植**: Rust 端将其视为异步外部服务调用,契约即上述入参与出参形状。

#### `_build_agent(workspace, *, system_prompt=None) -> BaseAgent`(行 2382-2410,LLM 驱动)
- `system_prompt` 默认 `DIAGNOSIS_SYSTEM_PROMPT`;传 `AGGREGATION_SYSTEM_PROMPT` 可切换(当前无调用)。
- `load_model_config_ref(ref_path)` → `TeamModelConfig.model_validate(without_inner_sdk_retries(model_data))` → `model.build()`。
- `create_deep_agent(model=..., card=AgentCard(name="diagnosis_agent", description="Evaluation result diagnosis agent"), system_prompt=..., workspace=workspace, restrict_to_work_dir=True, max_iterations=config.diagnosis_agent_max_iterations, auto_create_workspace=False, rails=[RSISysOperationRail(read_only=True, bash_pipefail=True)])`。

#### `_per_case_diagnosis(...) -> list[dict]`(行 2412-2546,LLM 驱动编排)

- `runtime_root = tempfile.mkdtemp(prefix="ach_analyzer_")`(行 2427)。
- **循环是顺序 await**(行 2541-2543:`for case in case_inputs: results.append(await _diagnose_one(case))`)——docstring 声称并发,但**实际串行**;`diagnosis_agent_max_concurrency` 未使用。Rust 移植可保留串行或按配置并发,但**默认行为必须等价**(结果顺序 = 输入顺序)。
- 每 case `_diagnose_one`(行 2429-2538):
  1. `runtime_dir = _make_diagnosis_runtime_dir(runtime_root, case.case_id)`(行 2430)= `runtime_root / f"{safe_case_id}-{uuid.uuid4().hex}"`(`_safe_path_segment` 见 §3.10,行 1437-1440)。
  2. `evidence_summary_available = _prepare_diagnosis_evidence(case, runtime_dir)`(行 2432-2435,确定性,见 §3.8)。
  3. `prompt = _build_diagnosis_prompt(case, signals, retrieved_experience, evidence_summary_available, source_stage, prior_candidate_feedback=_case_prior_candidate_feedback(prior_candidate_feedback, case.case_id))`(行 2436-2446,确定性拼装)。
  4. `agent = await self._build_agent(str(runtime_dir))`;`raw = await _run_agent(agent, prompt, max_retries=config.diagnosis_agent_max_retries)`(行 2447-2452,LLM)。
  5. `parsed = _extract_json_object(raw)`;若 None → 用 `_build_json_repair_prompt(prompt, raw)` 再跑一次(`max_retries=0`);仍 None → `raise ValueError(...)`,被 except 捕获(行 2453-2465)。
  6. `validation_inventory` / `verifier_inventory`(确定性,§3.7);`validation_conflicts = _diagnosis_validation_conflicts(parsed, validation_inventory, verifier_inventory)`(行 2466-2472)。
  7. 若有冲突 → 用 `_build_evidence_conflict_repair_prompt(...)` 再跑一次(`max_retries=0`);解析后重查冲突;修复成功则采用,否则保留(追加 "evidence-conflict repair output did not contain JSON")(行 2473-2502)。
  8. 冲突仍未消除 → 记 warning,返回 `_diagnosis_evidence_conflict_result(case, conflicts)`(行 2503-2512)。
  9. 成功 → 返回(行 2513-2526):
     ```json
     { "case_id", "score", "evaluation_passed", "evaluation_reason",
       ...parsed,   // 展开 LLM 输出的全部顶层字段
       "verifier_failure_output_excerpt": verifier_inventory["verifier_failure_output_excerpt"] or "" }
     ```
  10. `except Exception`(行 2527-2536): `is_retryable_model_call_failure(exc)` → 记 warning 并返回 `_diagnosis_unavailable_result(case, exc)`;否则 `logger.exception` 后 **re-raise**。
  11. `finally: _remove_path(runtime_dir)`(行 2537-2538)。
- 外层 `finally: _remove_path(runtime_root)`(行 2545-2546)。

#### `_aggregate_diagnosis(...) -> list[TeamIssue]`(行 2548-2569)
- **纯确定性**: `return _aggregate_structured_diagnoses(per_case_results=..., max_issues=config.max_issues, evidence_limit_per_issue=config.evidence_limit_per_issue)`(行 2565-2569)。
- 不构建 agent、不调用模型;`output_dir`/`source_stage` 参数当前未使用。

#### `_partial_artifact(invocation, reason, retrieved_experience)`(行 2571-2587)
- 返回 `analysis_status="partial"`、`failure_reason=reason`、`issues=[]`、`strategy=self.name` 的 artifact。

### 3.3 `_run_agent(agent, prompt, *, max_retries) -> str`(行 2595-2638,LLM 编排)

- `session_id = f"diagnosis_{uuid.uuid4().hex}"`(行 2604)。
- `call_once`(行 2607-2616): `Runner.run_agent(agent=..., inputs={"query": current_prompt}, session=session_id)`;
  结果若 dict → `str(result.get("output", result.get("answer", json.dumps(result, ensure_ascii=False))))`;否则 `str(result)`。
- `attempts = max(1, int(max_retries or 0) + 1)`;`model_call_retries = 1 if attempts > 1 else 0`(行 2619-2620)。
- 每轮: 用 `run_model_call_with_retries(call_once, operation_name="diagnosis agent", max_retries=model_call_retries)` 执行;
  `BaseException` 时若已是最后一轮或不可重试 → raise,否则 continue(行 2621-2634)。
- 成功输出 `_extract_json_object(last_raw) is not None` → 立即返回(行 2635-2636)。
- 否则 `current_prompt = _build_json_repair_prompt(prompt, last_raw)` 进入下一轮(行 2637)。
- 耗尽后返回最后一次 `last_raw`(行 2638)。

### 3.4 `_aggregate_structured_diagnoses(*, per_case_results, max_issues, evidence_limit_per_issue) -> list[TeamIssue]`(行 1077-1163,确定性)

规则引擎,逐条:
1. 分组键 `(target_ref, failure_mode)`(行 1084-1093):
   - 跳过 `analysis_failed` 为真的项(行 1086-1087)。
   - `target_ref = _normalize_target_ref(item.get("target_ref",""))`;空或 `"unassigned"` → 跳过(行 1088-1090)。
   - `issue_category = str(item.get("issue_category", item.get("category", "")) or "")`;`failure_mode = str(item.get("failure_mode","") or "")`(行 1091-1092)。
2. 排序(行 1095-1103): 键为 `(-max(severity_rank), -max(confidence_rank), target_ref, failure_mode)` —— 组内最高严重度/置信度降序,再按 target_ref、failure_mode 字典序升序。
3. 取前 `max(0, max_issues)` 组(行 1105,`enumerate(start=1)` → `issue_id = f"issue_{index:03d}"`,行 1136)。
4. `strongest = max(items, key=(severity_rank, confidence_rank))`(行 1106-1112,并列取首个)。
5. evidence 构建(行 1113-1122): 每组按 items 顺序:
   `{"case_id": str(item.get("case_id","")), "failure_mode": str(item.get("failure_mode", failure_mode)), "affected_component": components[0] if components else ""}`;
   最终 `evidence[:max(1, evidence_limit_per_issue)]`(**至少 1 条**,行 1142)。
6. `affected_components` = 保序去重(dict.fromkeys)所有 items 的 `_string_items(affected_components)`(行 1123-1127)。
7. `issue_category = _issue_category_from_target_ref(target_ref)`(行 1128): `"member_harness"`/`"team_skill"`/`""`。
8. `affected_cases` = items 顺序的 case_id(行 1129-1133)。
9. 构造 `TeamIssue`(经 `_dict_to_team_issue`,行 1134-1162):
   - `category = "member_harness" if issue_category == "member_harness" else "team_coordination"`
   - `severity = str(strongest.get("severity", "medium") or "medium")`
   - `summary`、`recommendation` 取自 strongest
   - `suspected_team_scope = "member" if issue_category == "member_harness" else "team_skill"`
   - `metadata.attribution`(全部取自 strongest): `root_cause`、`critical_mistake`、`general_mechanism`、
     `decision_contract`(非 dict 归一 `{}`)、`target_ref`、`evidence_refs`(list 或 `[]`)、`confidence`。
10. `issues.append(_apply_g5_mapping(issue))`(行 1162)。

### 3.5 G5 映射 `_apply_g5_mapping(issue) -> TeamIssue`(行 1237-1280,确定性)

- `target_ref = _issue_target_ref(issue)`(行 1244): `attribution.target_ref` 归一为小写、strip、`-`→`_`(行 1283-1288)。
- **排除门**: `target_ref == "unassigned"` 或 `_is_evidence_pipeline_failure(issue)` → `replace(optimization_target="", target_members=[])`(行 1245-1246)。
- `target_scope = _target_scope_from_target_ref(target_ref)`(行 1248):
  - `"member_harness"`(行 1249-1257): 先查 `_coordinator_member_issue_as_team_skill(issue, target_ref)`;命中则返回转换结果;
    否则 `optimization_target="member_harness"`、`target_members=_target_members_from_issue(issue)`;与现状相同则原样返回,否则 `replace`。
  - `"team_skill"`(行 1258-1263): `optimization_target="team_skill"`、`target_members=[]`。
  - **空 scope**(行 1265-1280): 回退到 `suspected_team_scope` / `category`:
    - `scope == "member"` 或 `category == "member_harness"` → member_harness + `_target_members_from_issue`
    - `elif scope == "team_skill"` 或 `category == "team_coordination"` → team_skill + `[]`
    - else → team_skill + `[]`(**兜底也是 team_skill**)
- 无变化时返回原 issue(保持 frozen dataclass 共享引用语义)。

#### 关联辅助(全部确定性)
- `_issue_target_ref(issue)`(行 1283-1288): 见上。
- `_coordinator_member_issue_as_team_skill(issue, target_ref)`(行 1291-1317):
  - `role = _target_member_from_target_ref(target_ref)`;`is_team_coordinator_role(role)` 为假 → 返回 `None`(行 1302-1304,依赖 `team_factory` 的协调者角色判定)。
  - 否则: `team_target_ref = _coordinator_team_skill_target_ref(issue)`;metadata 里写入新 target_ref;`affected_components` 为空则补 `["team_leader"]`;
    返回 `replace(issue, category="team_coordination", suspected_team_scope="team_skill", optimization_target="team_skill", target_members=[], metadata=...)`。
- `_coordinator_team_skill_target_ref(issue)`(行 1320-1322): `variable = "constraint_violation" if _looks_like_completion_contract_issue(issue) else "role_coordination"`;返回 `f"team_skill.team_leader.{variable}"`。
- `_looks_like_completion_contract_issue(issue)`(行 1325-1349): 把 summary/recommendation/evidence JSON/metadata JSON 全部小写拼接;命中任一标记(`artifact`、`claim_task`、`complete`、`completion`、`deliverable`、`file`、`output`、`required`、`status`、`verify`、`verification`)→ True。
- `_with_attribution_target_ref(metadata, target_ref)`(行 1352-1361): 浅拷贝 metadata,写 `attribution.target_ref`。
- `_is_evidence_pipeline_failure(issue)`(行 1364-1387): 拼接文本小写;必须**同时**命中证据产物标记(`trajectory_events.jsonl`、`normalized_trace.json`、`evidence_summary.md`)之一 **且** 缺失标记(`no such file`、`not found`、`missing`、`failed to read`、`failed to load`)之一。
- `_target_members_from_issue(issue)`(行 1390-1403): 候选来源依次: ① `issue.target_members` ② target_ref 中的成员 ③ `metadata.affected_components` ④ 每条 evidence 的 `affected_components`/`affected_component`;最后保序去重。
- `_target_scope_from_target_ref(target_ref)`(行 1406-1410): `split(".")` 首段 ∈ {member_harness, team_skill} 返回首段,否则 `""`。
- `_target_member_from_target_ref(target_ref)`(行 1413-1417): 段数 ≥3 且首段 member_harness 且第二段非空 → 第二段,否则 `""`。
- `_string_items(value)`(行 1420-1434): str → strip 后非空则 `[s]`;list → 过滤非 str、strip 后非空;否则 `[]`。
- `_normalize_target_ref(value)`(行 1215-1216): `str(value or "").strip().replace("-", "_")`。
- `_issue_category_from_target_ref(target_ref)`(行 1219-1221): scope ∈ {member_harness, team_skill} 返回 scope,否则 `""`。
- `_severity_rank` / `_confidence_rank`(行 1224-1229): `{"low":1, "medium":2, "high":3}.get(小写, 0)`。

### 3.6 失败结果构造(确定性)

- `_diagnosis_unavailable_result(case, exc) -> dict`(行 1166-1191): 固定字段集(见源码),`analysis_failed=True`、`diagnosis_status="unavailable"`、`issue_category="unassigned"`、`severity="low"`、
  `failure_mode="diagnosis_unavailable"`、`target_ref="unassigned"`、`confidence="low"`、`error=str(exc)`。
- `_diagnosis_evidence_conflict_result(case, errors) -> dict`(行 2056-2079): 同上结构,`diagnosis_status="evidence_conflict"`、`failure_mode="diagnosis_evidence_conflict"`、`root_cause="; ".join(errors)`、`summary="Diagnosis contradicted deterministic evaluation evidence."`。

### 3.7 确定性清单/冲突检查(analyzer.py 内部)

#### `_build_validation_inventory(case) -> dict`(行 1733-1740)
- 读 `case_dir / "judge" / "normalized_trace.json"` → `_summarize_normalized_trace`;再 `_validation_inventory_from_events(_validation_events_from_result(case.result_path) or trace_events, verifier_passed=case.evaluation_passed)`。

#### `_build_verifier_inventory(case) -> dict`(行 1743-1779)
- `empty_patch = metadata.get("empty_patch")`;`instance_report = metadata.get("instance_report")`;非 dict 或空 → `{"empty_patch": ...}`(仅当 empty_patch 非 None)或 `{}`。
- `report = raw_reports.get(case.case_id)`,非 dict → 取第一个 dict 值;仍无 → `{}`。
- `_failures(group_name)`: `tests_status[group]["failure"]` list 取前 24 个非空 str(行 1759-1766)。
- 返回(行 1768-1779): `{"empty_patch", "patch_exists", "patch_successfully_applied", "resolved", "failed_fail_to_pass_tests": _failures("FAIL_TO_PASS"), "failed_pass_to_pass_tests": _failures("PASS_TO_PASS"), "verifier_failure_output_excerpt": _truncate_text(str(metadata.get("test_output_excerpt") or ""), 8000)}`。

#### `_validation_events_from_result(result_path) -> list[dict]`(行 1782-1811)
- `metadata.execution.command_log` list;逐记录:
  - `output = "\n".join(stdout_excerpt, stderr_excerpt 的非空部分)`。
  - `exit_code in {None, 0, "0"}` → `error=""`,否则 `error=f"exit_code={exit_code}"`(注意 `0 == "0"` 在 Python 为 False,集合成员测试用 `in` 集合,`exit_code=0` 命中 0,`exit_code="0"` 命中 "0",`0.0` 与 0 相等也命中)。
  - 事件: `{"tool": "command_log", "input": str(command), "output": _one_line(output, 300), "output_tail": _one_line(output[-1200:], 1200), "error": error, "validation_result": _validation_result_signal(output, error)}`。

#### `_validation_inventory_from_events(events, *, verifier_passed) -> dict`(行 1814-1853)
- 项目测试判定(行 1822-1827): `lowered = " ".join(command.lower().split())`;
  `is_pytest_suite = "pytest" in lowered and any(marker in f" {lowered} " for marker in (" tests/ ", " ./tests/ "))`;
  `is_project_suite = is_pytest_suite or any(marker in lowered for marker in ("make test", " tox", "tox ", " nox", "nox ", "npm test", "pnpm test"))`。
- `result = str(event.get("validation_result") or "unknown")`。
- `suite_result`(行 1839-1847): 无事件 → `"not_observed"`;任一 `"passed"` → `"passed"`;否则任一 `"failed"` → `"failed"`;否则 `"not_observed"`(**passed 优先于 failed**)。
- 返回(行 1848-1853): `{"project_test_suite_attempted": bool(project_events), "project_test_suite_result": suite_result, "authoritative_verifier_result": "passed" if verifier_passed else "failed", "project_test_events": project_events[-4:]}`。

#### `_validation_result_signal(output, error) -> str`(行 2150-2161)
- `error.strip()` 非空 → `"failed"`。
- `re.findall(r"\b(\d+)\s+failed\b", output.lower())` 任一计数 > 0 → `"failed"`。
- `re.findall(r"\b(\d+)\s+passed\b", ...)` 任一 > 0 → `"passed"`。
- 否则 `"unknown"`。

#### `_summarize_normalized_trace(trace_data) -> list[dict]`(行 2090-2147)
- `traces` list;逐 trace(`trace_id`、`role = str(trace.get("member_role", trace.get("role", "")))`);逐 message:
  - `content` 非空 → 事件 `{"trace_id", "role", "message_index", "step_pointer": str(...), "tool": "", "input": "", "output": _one_line(content, 500), "error": ""}`(行 2108-2121)。
  - `tool_calls` list → 每 call 事件(行 2122-2146): `{"trace_id", "role", "message_index", "step_pointer": str(call.get("step_pointer","")), "tool": str(call.get("name","")), "input": _one_line(call.get("input",""), 300), "output": _one_line(raw_output, 300), "output_tail": _one_line(raw_output[-300:], 300), "error": _one_line(raw_error, 500), "validation_result": _validation_result_signal(raw_output, raw_error)}`。
- 保持遍历顺序(确定性展平)。

#### `_diagnosis_validation_conflicts(diagnosis, inventory, verifier_inventory=None, *, public_task=None) -> list[str]`(行 1856-2033)

返回错误字符串列表;空列表 = 无冲突。规则(按顺序):

1. **本地通过 vs 验证器失败**(行 1865-1891): 若 `project_test_suite_result == "passed"` 且 `authoritative_verifier_result == "failed"`:
   - `validation_observations` 必须为 dict,否则报 `"missing validation_observations for local-pass/verifier-fail contradiction"`。
   - 三个键必须精确相等: `project_test_suite_attempted == True`、`project_test_suite_result == "passed"`、`authoritative_verifier_result == "failed"`(逐键报 `validation_observations.<key> must equal <repr>` 格式,注意 repr 带引号,如 `'passed'`)。
   - `contradiction_explanation` strip 后非空。
   - recommendation 归一化小写后不得含: `"require running the project's existing test suite"`、`"must run the project's own test suite"`、`"rather than only a self-authored smoke"`、`"instead of only a self-authored smoke"`。
2. **empty_patch**(行 1897-1942): `verifier_inventory.get("empty_patch") is True` 时:
   - 若 `activation_phase ∈ {post_diagnosis, pre_submission}` 且 `target_ref != "unassigned"` → 报错。
   - `edit_was_already_justified` = 诊断全文含任一句式(行 1906-1918 列表);`direct_edit_action` = required_action 小写含任一动作短语(行 1919-1930 列表)。
   - 若两者都为真且 `activation_phase ∉ {post_diagnosis, pre_submission}` → 报错;若 `target_ref != "unassigned"` 且 activation_phase 同样越界 → 再报错。
3. **补丁已应用但未解决**(行 1944-1966): `patch_successfully_applied is True` 且 `resolved is False` 时:
   - `verifier_observations` 必须为 dict;三个键精确相等: `patch_successfully_applied == True`、`failed_fail_to_pass_tests`、`failed_pass_to_pass_tests`(与 inventory 值全等比较)。
   - 诊断全文不得含补丁失败声称(行 1958-1964 列表)。
4. **test_next 迭代器协议**(行 1968-1999): `failed_fail_to_pass_tests` 中存在含 `"test_next"` 的测试名(小写)且前面无冲突时:
   - 协议文本必须含 `("__next__" | "direct next" | "next(")` 之一 **且** 生命周期词之一(`state`、`lifecycle`、`initializ`、`exhaust`、`stopiteration`、`transition`)。
   - 若 verifier 输出含 `"attributeerror"` 或 `"not initialized"` → 协议文本还须含预初始化词之一(`attributeerror`、`before iter`、`before __iter__`、`pre-init`、`preinit`、`uninitialized`、`initializ`)。
5. **safe file-replacement**(行 2001-2032): `failed_fail_to_pass_tests` 中含同时含 `"safe"` 与 `"replace"` 的测试名且前面无冲突时:
   - `decision_contract` 的 `causal_distinction`、`required_action`、`acceptance_observable`、`scope_boundary`(list → join)拼接小写后须含事务词之一(`atomic`、`existing file`、`original file`、`preserve the old`、`preserve the original`、`temporary file`、`temp file`、`replace only after`、`unchanged on failure`)。

辅助:
- `_joined_diagnosis_text(diagnosis)`(行 2036-2046): 小写拼接 `summary`、`root_cause`、`critical_mistake`、`general_mechanism`、`recommendation`,空格连接。
- `_contains_any_phrase(text, phrases)`(行 2049-2053): 任一子串命中。

### 3.8 evidence 构建与运行时目录(确定性)

- `_make_diagnosis_runtime_dir(runtime_root, case_id)`(行 1437-1440): `runtime_root / f"{_safe_path_segment(case_id) or 'case'}-{uuid.uuid4().hex}"`。
- `_DIAGNOSIS_COPY_IGNORES`(行 1443-1452): `{".git", ".mypy_cache", ".pytest_cache", ".ruff_cache", ".tox", ".venv", "__pycache__", "node_modules"}`。
- `_DIAGNOSIS_ROOT_RUNTIME_DIRS`(行 1453-1459): `{"agents", "context", "memory", "messages", "todo"}`(仅工作区根目录忽略)。
- `_load_case_result(case) -> dict`(行 1462-1471): 读 result.json;非文件/解析失败/非 dict → `{}`。
- `_prepare_repository_snapshot(*, case, runtime_dir) -> bool`(行 1474-1527):
  - `result.get("workspace_dir")` 非空 str 且目录存在,否则 False。
  - `copytree(workspace_dir → runtime_dir/"repository", ignore=_ignore_runtime_noise, symlinks=False)`;OSError → warning + `_remove_path` + False(行 1497-1511)。
  - `evaluation.metadata.model_patch_path` 非空 str 且文件存在 → `copy2` 到 `runtime_dir / "source_patch.diff"`(行 1513-1526)。
  - 返回 True。
- `_prepare_diagnosis_evidence(*, case, runtime_dir) -> bool`(行 1530-1539): `_remove_path(runtime_dir)` → `mkdir(parents=True, exist_ok=True)` → `_prepare_repository_snapshot(...)` → `summary = _build_evidence_summary(case)`;`summary.strip()` 为空 → False;写 `runtime_dir/"evidence_summary.md"` → True。
- `_build_evidence_summary(case) -> str`(行 1542-1730): 拼接 Markdown(章节依次):
  1. `# Analyzer Evidence Summary` + `## Authoritative Task Contract`(`case.input` 原文,行 1548-1558)
  2. `## Authoritative Benchmark Test Contract`(可选,含 fail_to_pass/pass_to_pass 的 JSON 与 test_patch 的 diff 块,行 1559-1588)
  3. `## Case Facts`(case_id/status/score/evaluation_passed/evaluation_method/evaluation_reason,可选 execution_error,行 1589-1602)
  4. `## Judge Quality Gaps`(前 8 条,行 1604-1622)
  5. `## Low-Score Judge Behaviors`(score < 0.8 的前 8 条,`_safe_float` 解析,行 1624-1639)
  6. `## Deterministic Validation Inventory`(行 1641-1666): 从 `judge/normalized_trace.json` + result.json 的 command_log;含 `project_test_suite_attempted/result`、`authoritative_verifier_result`、project_test_event 行(输出取 `output[-700:]` 前 700);本地通过而验证器失败时加 `- hard_fact: ...` 行。
  7. `## Deterministic Verifier Inventory`(行 1668-1701): empty_patch、patch_successfully_applied、resolved、failed_fail_to_pass_tests、failed_pass_to_pass_tests;`verifier_failure_output_excerpt` 截断 3500 的 text 块;应用成功未解决时加 hard_fact 行。
  8. `## Verifier Outcome`(行 1703-1713): `verifier/reward.txt`(80 字符)、`stderr.log`/`stdout.log`(各 1500)。
  9. `## Agent-Generated Execution Evidence`(行 1715-1728): 失败步骤前 8 条(`_format_trace_event`)、Key Events 最后 12 条。
  - 返回 `"\n".join(lines).strip() + "\n"`(行 1730)。
- `_format_trace_event(event)`(行 2164-2179): `"- [err|ok] trace_id=... role=... message_index=... step=... tool=..."` + 条件附加 input/error/output。
- `_safe_path_segment(value)`(行 2205-2208): 仅保留 `isalnum()` 或 `-_.` 的字符,其余 → `_`;结果 `strip("._")`;空结果由调用方回退 `"case"`。
- `_remove_path(path)`(行 2211-2218): 文件 unlink(missing_ok=True);目录 `shutil.rmtree(ignore_errors=True)`。

### 3.9 提示词构建与有界化(确定性)

- `_build_diagnosis_prompt(*, case, signals, retrieved_experience, evidence_summary_available, source_stage="", prior_candidate_feedback=None)`(行 582-625):
  - 依 `evidence_summary_available` 选 `evidence_instruction` 两套文案之一(行 597-612)。
  - `PER_CASE_DIAGNOSIS_TEMPLATE.format(stage_instruction=_stage_instruction(source_stage), evidence_instruction=..., diagnosis_input=_build_diagnosis_input_json(...))`(行 614-625)。
- `_build_diagnosis_input_json(...) -> str`(行 628-709): 组装 payload 后 `json.dumps(payload, ensure_ascii=False, indent=2)`(行 709)。payload 结构(键序固定):
  1. `authoritative_task_contract`: `{"provenance": "case.input", "input_excerpt": case.input, "policy": <固定文案>}`(行 645-653)
  2. `authoritative_benchmark_test_contract`: `case.benchmark_test_contract`(行 654)
  3. `primary_evidence`: `{"evidence_summary_available", "evidence_summary_path": "evidence_summary.md"|"", "evidence_summary_text": <有界 6000>}`(行 655-659)
  4. `deterministic_validation_inventory`(行 660)
  5. `deterministic_verifier_inventory`(行 661)
  6. `prior_candidate_feedback`: `_bounded_structured_value(prior_candidate_feedback or {}, 8000)`(行 662-665)
  7. `prior_candidate_feedback_policy`: 固定文案(行 666-671)
  8. `analysis_stage`: `source_stage or "unknown"`(行 672)
  9. `anchor_signals`: `{"method", "exec_failures": _case_scoped_list(..., case_id), "judge_failures": ..., "error_clusters": _bounded_structured_value(_case_scoped_error_clusters(...), 2500), "method_specific": _bounded_structured_value(_case_scoped_method_specific(...), 2500)}`(行 673-685)
  10. `case_facts`: `{"case_id", "status", "score", "evaluation_passed", "evaluation_reason": _truncate_text(...,1200), "error": _truncate_text(...,1200), "judge_breakdown": _summarize_evaluation_metadata(...), "training_signal": _bounded_structured_value(...,2000)}`(行 686-698)
  11. `fallback_excerpts`: `{"input_excerpt": case.input, "response_excerpt": _truncate_text(case.response, 1200)}`(行 699-702)
  12. `retrieved_experience`: `_bounded_structured_value(_compact_retrieved_experience(retrieved_experience), 2000)`(行 703-706)
  13. `experience_usage_policy`: `_experience_usage_policy()`(行 707)
- `_stage_instruction(source_stage)`(行 750-777): 四个分支 —— `"single_harness_candidate_failure"`、`"member_stage"`、`"team_skill_stage"`、默认。
- `_truncate_text(value, limit)`(行 780-786): `text = str(value or "")`;超长 → `f"{text[:limit]}\n...[truncated {len(text)-limit} chars]"`。
- `_bounded_json(value, limit=_SIGNAL_SNIPPET_CHARS)`(行 789-791): `_truncate_text(json.dumps(value, ensure_ascii=False), limit)`。
- `_bounded_structured_value(value, limit)`(行 794-799): JSON 长度 ≤ limit → 原值;否则 `{"truncated_json": _truncate_text(text, limit)}`。
- `_case_scoped_list(items, case_id)`(行 802-804): `[case_id] if case_id in items else []`。
- `_case_scoped_error_clusters(clusters, case_id)`(行 807-819): 仅保留含该 case 的簇,`cases` 重写为 `[case_id]`。
- `_case_scoped_method_specific(metadata, case_id)`(行 822-827): 逐键 `_case_scoped_value`。
- `_case_scoped_value(value, case_id)`(行 830-841):
  - dict: 含 case_id 键 → `{case_id: value[case_id]}`;否则递归过滤出"提及 case"的键值对。
  - list: 含 case_id → `[case_id]`;否则过滤 dict 项(须 `_value_mentions_case`)。
  - 标量: 原样。
- `_value_mentions_case(value, case_id)`(行 844-849): 递归;标量 `value == case_id`。
- `_summarize_evaluation_metadata(metadata)`(行 852-905): 见 §3.11。
- `_compact_retrieved_experience(retrieved_experience | None)`(行 990-1018): falsy → `{}`;`matches` 前 3 条,每条 `{"experience_id": item.get("experience_id", item.get("id","")), "component_layer", "failure_signature", "mechanism_type", "learning_status", "summary": _truncate_text(item.get("summary", item.get("content","")), 500), "experience": _bounded_structured_value(item.get("experience",{}), 800), "metadata": item.get("metadata",{})}`;返回 `{"stage", "matches", "metadata"}`。
- `_experience_usage_policy()`(行 1021-1036): 固定结构化规则 dict(见源码,移植原样)。
- `_build_aggregation_prompt(...)`(行 712-747): **死代码**,但模板替换规则已列于 §3.1;若保留需逐占位符一致。其中 `per_case_diagnoses` 经 `_compact_per_case_diagnoses` 后以 `_bounded_json(..., 1200 * max(1, len))` 限制。
- `_compact_per_case_diagnoses(per_case_diagnoses)`(行 1039-1074): **死代码**(仅被 `_build_aggregation_prompt` 使用);把扁平归因字段(每项截断 500)打包进 `attribution` 子字典,顶层保留 `case_id/analysis_failed/issue_category/severity/summary/failure_mode/affected_components/recommendation`。
- `_case_prior_candidate_feedback(feedback, case_id)`(行 1194-1212): `feedback["by_case"][case_id]`(dict 包装为单元素列表)取**最后 3** 条 dict 记录;返回 `{"case_id", "experiments": [...]}`;各步类型不符 → `{}`。

### 3.10 JSON 提取 `_extract_json_object(text) -> dict | None`(行 2226-2243)
- `text.find("{")` 无 → None;从首个 `{` 起括号深度扫描;深度归零处 `json.loads` 尝试,失败 → None;未闭合 → None。
- 与 `scoring.py parse_judge_output` 的括号扫描一致。

### 3.11 `_summarize_evaluation_metadata` 细节(行 852-905,确定性)

- `parsed = metadata.get("parsed", {})`;非 dict → `{}`。
- `behaviors` list 逐 dict(行 866-879): `{"id": entry.get("id",""), "score": entry.get("score"), "reason": _truncate_text(...,1200), "failure_reason": ..., "missing_capability": ..., "suggested_surface_hint": _truncate_text(...,80), "evidence": entry.get("evidence","")}`。
- `quality_gaps = _compact_quality_gaps(parsed.get("quality_gaps", []))`(行 883)。
- `dataset_budget = _compact_dataset_budget(parsed.get("dataset_budget", {}))`(行 884)。
- `dimensions = _compact_judge_dimensions(parsed.get("dimensions", {}))`(行 885)。
- **判空**(行 887-888): `not any((behaviors, overall_reason, forbidden_hits, quality_gaps))` → `{}`(注意 dataset_budget/dimensions 不参与判空)。
- 结果(行 890-905): `{"behaviors", "overall_reason": _truncate_text(...,1200), "forbidden_hits": list 检查}` + 可选 `quality_gaps`/`dataset_budget`/`dimensions` + `quality_gap_score_ceiling`/`overall_score`(若键存在于 parsed)。
- `_compact_quality_gaps(value)`(行 908-940): list 前 8 条;`gap_type.lower() == "verification_gap"` → **跳过**(行 922-924);其余 11 字段(见源码;affected_roles/likely_surfaces 限 8;文本字段 _truncate_text 1200)。
- `_compact_dataset_budget(value)`(行 943-966): `case_groups` 前 8 条 × 4 字段;`total_cases` 若存在。
- `_compact_judge_dimensions(value)`(行 969-980): 5 个标量键若存在;`per_behavior_scores` dict 前 12 项 `{str(key): score}`。
- `_safe_float(value)`(行 2082-2087): `float(value)` 失败 → `1.0`(**注意默认值 1.0 而非 0.0**)。
- `_one_line(value, limit)`(行 2182-2183): `" ".join(_truncate_text(value, limit).split())`。
- `_read_text_if_exists(path, limit)`(行 2186-2192): 读文件 `errors="replace"`,截断;OSError → ""。
- `_read_json_if_exists(path)`(行 2195-2202): 读 JSON,非 dict/解析失败 → `{}`。

### 3.12 门面 `EvaluationResultAnalyzer`(行 2767-2806)

- `__init__(config, experience_learner=None)`(行 2770-2776): `self.config = config`;`self._strategy = build_analysis_strategy(config, experience_learner)`。
- `async analyze(invocation) -> str`(行 2778-2806):
  1. `output_dir = Path(invocation.output_dir).expanduser().resolve()`;`mkdir(parents=True, exist_ok=True)`(行 2787-2788)。
  2. `issues_path = output_dir / config.output_filename`(**默认 "issues.yaml"**);`analysis_ref_path = output_dir / "analysis_ref.yaml"`(行 2789-2790)。
  3. `artifact = await self._strategy.analyze(invocation)`(行 2792)。
  4. `issues_dicts = [_backfill_issue_evidence_refs(asdict(issue), invocation) for issue in artifact.issues]`(行 2794,`dataclasses.asdict` 深转 dict)。
  5. `_write_yaml(issues_path, {"issues": issues_dicts})`(行 2795)。
  6. `_write_yaml(analysis_ref_path, _build_analysis_ref_dict(...))`(行 2796-2805)。
  7. 返回 `str(analysis_ref_path)`(行 2806)。

#### 门面私有辅助(确定性)
- `_build_analysis_ref_dict(*, output_dir, invocation, issues_path, issues_dicts, artifact_metadata)`(行 2814-2837):
  - `retrieved_experience` 从 metadata 提出为**顶层键**(向后兼容);`core_metadata` 为去掉该键的其余。
  - 返回: `{"analysis_id": output_dir.name, "created_at": datetime.now(UTC).astimezone().isoformat(), "source_eval_ref_path", "case_results_dir", "case_traces_dir", "team_skill_ref_path", "harness_refs_path", "issues_path": str(...), "issues": issues_dicts, "retrieved_experience": ..., "metadata": core_metadata}`。
- `_backfill_issue_evidence_refs(issue_dict, invocation)`(行 2840-2863):
  - `attribution.evidence_refs` 非空 list → 原样返回。
  - 否则按 `affected_cases`(过滤空白)查 `_case_artifact_index`;无命中 → 原样;命中 → `attribution["evidence_refs"] = refs[:3]`。
- `_case_artifact_index(case_results_dir)`(行 2866-2894): 解析根目录后 `sorted(glob("*/result.json"))`;每 case:
  - `case_id = str(result.get("case_id","") or "").strip()`;空 → 跳过。
  - `ref = {"case_id": ..., "result_path": str(result_path.resolve())}`;`trace.json` 存在加 `trace_path`;`judge/normalized_trace.json` 存在加 `normalized_trace_path`。
  - `index.setdefault(case_id, ref)`(**首见保留**)。
- `_write_yaml(path, payload)`(行 2897-2901): `yaml.safe_dump(payload, allow_unicode=True, sort_keys=False)`。
- `_write_json(path, payload)`(行 2904-2908): `json.dumps(payload, ensure_ascii=False, indent=2)`。
- `_dict_to_team_issue(data) -> TeamIssue`(行 2692-2735): `category` 白名单 `{member_harness, team_coordination}`,否则 `team_coordination`(行 2699-2700);
  attribution 取数顺序: `metadata["attribution"]` → `data["metadata"]["attribution"]` → 顶层扁平键(root_cause/critical_mistake/general_mechanism/decision_contract/target_ref/evidence_refs/confidence)(行 2703-2719);
  `affected_components` 非空时写回 `metadata["affected_components"]`(行 2721-2723);
  字段默认: `issue_id = str(data.get("issue_id", f"issue_{id(data)}"))`(Python `id()` 内存地址,移植需替换为稳定唯一 id 并文档化差异)、`severity="medium"`、`suspected_team_scope="both"`(行 2724-2735)。
- `build_analysis_strategy(config, experience_learner=None)`(行 2743-2759): 直接构造 `DiagnosisAgentStrategy(config, experience_learner)`;模型配置缺失也不抛错(延迟到 analyze)。

---

## 4. 确定性 vs LLM 驱动 划分总表

| 模块/函数 | 性质 | 移植说明 |
|---|---|---|
| `CaseReader` 全部方法与 §1.5 辅助 | **确定性** | 纯文件/JSON/YAML 读取 + 有界化,1:1 |
| `_fingerprint_error` 与 5 个正则 | **确定性** | regex crate + `split_whitespace().join(" ")` |
| 5 个 SignalExtractor 的 `extract` | **确定性** | 纯数据流;注意各提取器的 exec/judge 判定差异与 method_specific 合并差异 |
| 注册表 `register/build_signal_extractor` | **确定性** | 进程内全局可变 Map;Rust 用 `Mutex<HashMap<String, Box<dyn Extractor>>>` 或 `OnceLock` + 注册 API |
| `_normalized_trace_attribution`/`_normalized_trace_role`/`_behavior_diagnostics`/`_bounded_recent_events`/`_string_list` | **确定性** | 注意 `!r`(Python repr)需手写等价 |
| `_aggregate_structured_diagnoses` + G5 映射全家 | **确定性** | 分组/排序/最强项选择/白名单映射,1:1 |
| 冲突检查 `_diagnosis_validation_conflicts` | **确定性** | 纯文本规则引擎,1:1(短语列表逐字复制) |
| 提示词拼装 `_build_diagnosis_prompt`/`_build_diagnosis_input_json`/`_stage_instruction` 等 | **确定性** | 模板 + 有界 JSON 序列化;`ensure_ascii=False`、`indent=2`、键序需保持一致 |
| evidence 构建 `_build_evidence_summary` 等 | **确定性** | Markdown 拼装,1:1 |
| `_extract_json_object` | **确定性** | 括号扫描 + JSON 解析 |
| `_retrieve_experience` | **LLM/检索服务** | Rust 端保留异步调用契约(入参/出参形状),提示词不含 LLM 生成 |
| `_build_agent` / `_run_agent` / per-case 诊断循环 | **LLM 驱动** | 编排是确定性的(重试、JSON 修复、冲突修复),但模型输出不确定;Rust 需对接等价 agent/LLM 服务,提示词常量原样复制 |
| 聚合阶段 | **当前为确定性**(`_aggregate_structured_diagnoses`)| **不得**按旧设计调用 LLM;AGGREGATION_* 常量与 `_build_aggregation_prompt`/`_compact_per_case_diagnoses` 是死代码 |
| 门面写出 issues.yaml / analysis_ref.yaml | **确定性** | YAML dump(allow_unicode, sort_keys=False) |

---

## 5. 错误行为汇总

| 场景 | Python 行为 | Rust 移植要求 |
|---|---|---|
| `read_eval_ref` 路径不存在 | `raise ValueError("eval_ref path not found: <path>")` | 返回错误/panic 等价信息 |
| `read_summary` 路径为空或文件缺失 | 返回全默认 `EvaluationSummaryInput()` | 同上,不报错 |
| `read_summary` JSON 损坏 | 向上抛 `JSONDecodeError` | 报错 |
| `read_case_inputs` 目录不存在 | 返回 `[]` | 同上 |
| `register_signal_extractor` 空方法名 | `ValueError` | 报错 |
| `register_signal_extractor` 无 extract 方法 | `TypeError` | 报错 |
| `LlmJudgeSignalExtractor` 遇到 `avg_behavior_score=None` 等 | `float(None)` 抛 `TypeError` | 保持一致或显式处理并文档化 |
| per-case 诊断模型输出无 JSON(修复后仍无) | `ValueError` → 若 `is_retryable_model_call_failure` 则降级 `_diagnosis_unavailable_result`,否则 re-raise | 等价 |
| per-case 诊断与确定性证据冲突(修复后仍冲突) | 返回 `_diagnosis_evidence_conflict_result`,不抛错 | 等价 |
| `model_config_ref` 缺失 | `_partial_artifact("model_config_ref must be set")` | 等价 |
| 无 case_inputs | `analysis_status="empty_case_results"` 空 issues artifact | 等价 |

---

## 6. 关键行号索引

### 6.1 interfaces.py
| 功能 | 行号 |
|---|---|
| `SignalExtractor` Protocol(name/extract) | 21-35(24-27 / 29-35) |
| `EvaluationResultAnalysisStrategy` Protocol(name/analyze) | 38-57(47-50 / 52-57) |
| `__all__` | 60-62 |

### 6.2 case_reader.py
| 功能 | 行号 |
|---|---|
| `EvaluationSummaryInput` | 15-23 |
| `DeterministicSignals` | 26-34 |
| `CaseAnalysisInput` | 37-57 |
| `CaseReader.read_eval_ref` | 63-79 |
| `CaseReader.read_summary`(G4 映射) | 81-107(101-107) |
| `CaseReader.read_case_inputs` | 109-176 |
| `_read_benchmark_test_contract` | 179-221 |
| `_read_dataset_cases` | 224-239 |
| `_test_id_list` | 242-251 |
| `_bounded_trajectory_window_summary` | 254-277 |
| `_bounded_normalized_trace_summary` | 280-325 |
| `_resolve_normalized_trace_path` | 328-335 |
| `_bounded_tool_calls` | 338-354 |
| `_string_list` | 357-360 |
| `_excerpt` | 363-366 |

### 6.3 signal_extractor.py
| 功能 | 行号 |
|---|---|
| 5 个正则模式(_PATH/_HEX/_LINE/_TIMESTAMP/_UUID) | 24-28 |
| `_fingerprint_error`(替换顺序 + 空白归一) | 31-38(33-37 / 38) |
| `GenericSignalExtractor`(name/extract) | 46-103(49 / 51-103) |
| `PytestSignalExtractor`(name/extract/回退) | 111-180(119 / 121-180 / 158-169) |
| `RewardSignalExtractor`(name/extract) | 188-250(191 / 193-250) |
| `AtomicChecksSignalExtractor`(name/extract) | 258-336(261 / 263-336) |
| `LlmJudgeSignalExtractor`(name/extract/回退) | 344-426(352 / 354-426 / 397-408) |
| `_EXTRACTOR_MAP` 注册表 | 433-445 |
| `register_signal_extractor` | 448-455 |
| `build_signal_extractor` | 458-477 |
| `_string_list` | 480-483 |
| `_bounded_recent_events` | 486-499 |
| `_normalized_trace_attribution` | 502-555 |
| `_normalized_trace_role` | 558-562 |
| `_behavior_diagnostics` | 565-579 |

### 6.4 analyzer.py —— 常量与提示词
| 功能 | 行号 |
|---|---|
| 截断常量 | 77-85 |
| `DIAGNOSIS_SYSTEM_PROMPT` | 92-443 |
| `PER_CASE_DIAGNOSIS_TEMPLATE` | 445-458 |
| `AGGREGATION_SYSTEM_PROMPT` | 460-544 |
| `AGGREGATION_TEMPLATE` | 546-579 |

### 6.5 analyzer.py —— 提示词拼装与有界化
| 功能 | 行号 |
|---|---|
| `_build_diagnosis_prompt` | 582-625 |
| `_build_diagnosis_input_json` | 628-709 |
| `_build_aggregation_prompt`(死代码) | 712-747 |
| `_stage_instruction` | 750-777 |
| `_truncate_text` / `_bounded_json` / `_bounded_structured_value` | 780-786 / 789-791 / 794-799 |
| `_case_scoped_list` / `_case_scoped_error_clusters` / `_case_scoped_method_specific` | 802-804 / 807-819 / 822-827 |
| `_case_scoped_value` / `_value_mentions_case` | 830-841 / 844-849 |
| `_summarize_evaluation_metadata` | 852-905 |
| `_compact_quality_gaps` / `_compact_dataset_budget` / `_compact_judge_dimensions` | 908-940 / 943-966 / 969-980 |
| `_string_list`(analyzer 版) | 983-987 |
| `_compact_retrieved_experience` / `_experience_usage_policy` | 990-1018 / 1021-1036 |
| `_compact_per_case_diagnoses`(死代码) | 1039-1074 |
| `_case_prior_candidate_feedback` | 1194-1212 |

### 6.6 analyzer.py —— 主流程类
| 功能 | 行号 |
|---|---|
| `DiagnosisAgentStrategy.name` | 2265 |
| `DiagnosisAgentStrategy.__init__` | 2267-2276 |
| `DiagnosisAgentStrategy.analyze`(两阶段编排) | 2278-2366 |
| `_retrieve_experience` | 2368-2380 |
| `_build_agent` | 2382-2410 |
| `_per_case_diagnosis`(含 `_diagnose_one` 与修复循环) | 2412-2546(2429-2538) |
| `_aggregate_diagnosis`(确定性聚合入口) | 2548-2569 |
| `_partial_artifact` | 2571-2587 |
| `_run_agent` | 2595-2638 |
| `_build_json_repair_prompt` / `_build_evidence_conflict_repair_prompt` | 2641-2656 / 2659-2689 |
| `build_analysis_strategy` | 2743-2759 |
| `EvaluationResultAnalyzer.__init__` / `analyze` | 2770-2776 / 2778-2806 |

### 6.7 analyzer.py —— 确定性聚合与 G5
| 功能 | 行号 |
|---|---|
| `_aggregate_structured_diagnoses` | 1077-1163 |
| `_diagnosis_unavailable_result` | 1166-1191 |
| `_normalize_target_ref` / `_issue_category_from_target_ref` | 1215-1216 / 1219-1221 |
| `_severity_rank` / `_confidence_rank` | 1224-1225 / 1228-1229 |
| `_apply_g5_mapping` | 1237-1280 |
| `_issue_target_ref` | 1283-1288 |
| `_coordinator_member_issue_as_team_skill` / `_coordinator_team_skill_target_ref` | 1291-1317 / 1320-1322 |
| `_looks_like_completion_contract_issue` / `_with_attribution_target_ref` | 1325-1349 / 1352-1361 |
| `_is_evidence_pipeline_failure` | 1364-1387 |
| `_target_members_from_issue` / `_target_scope_from_target_ref` / `_target_member_from_target_ref` | 1390-1403 / 1406-1410 / 1413-1417 |
| `_string_items` | 1420-1434 |
| `_dict_to_team_issue` | 2692-2735 |

### 6.8 analyzer.py —— 证据/清单/冲突
| 功能 | 行号 |
|---|---|
| `_make_diagnosis_runtime_dir` / 忽略集合 | 1437-1440 / 1443-1459 |
| `_load_case_result` / `_prepare_repository_snapshot` / `_prepare_diagnosis_evidence` | 1462-1471 / 1474-1527 / 1530-1539 |
| `_build_evidence_summary` | 1542-1730 |
| `_build_validation_inventory` / `_build_verifier_inventory` | 1733-1740 / 1743-1779 |
| `_validation_events_from_result` / `_validation_inventory_from_events` | 1782-1811 / 1814-1853 |
| `_diagnosis_validation_conflicts` | 1856-2033 |
| `_joined_diagnosis_text` / `_contains_any_phrase` | 2036-2046 / 2049-2053 |
| `_diagnosis_evidence_conflict_result` / `_safe_float` | 2056-2079 / 2082-2087 |
| `_summarize_normalized_trace` / `_validation_result_signal` / `_format_trace_event` | 2090-2147 / 2150-2161 / 2164-2179 |
| `_one_line` / `_read_text_if_exists` / `_read_json_if_exists` | 2182-2183 / 2186-2192 / 2195-2202 |
| `_safe_path_segment` / `_remove_path` | 2205-2208 / 2211-2218 |
| `_extract_json_object` | 2226-2243 |

### 6.9 analyzer.py —— 门面写出
| 功能 | 行号 |
|---|---|
| `_build_analysis_ref_dict` | 2814-2837 |
| `_backfill_issue_evidence_refs` / `_case_artifact_index` | 2840-2863 / 2866-2894 |
| `_write_yaml` / `_write_json` | 2897-2901 / 2904-2908 |
| `__all__` | 2911-2914 |

---

## 7. 移植注意事项(易错点清单)

1. **正则等价**: Python `re` 与 Rust `regex` 语法基本兼容;`_PATH_PATTERN` 中 `[^\s,;'"\]]` 的转义需小心;Rust 字符串字面量转义(`\\`、`\"`)易错,建议用原始字符串 `r"..."`。
2. **`re.sub` 语义**: 非重叠、最左、全局替换;Rust `regex::Regex::replace_all` 等价。
3. **`" ".join(text.split())`**: Rust 用 `split_whitespace()`(会去除各类 Unicode 空白;Python `str.split()` 默认同样按 Unicode 空白)。
4. **dict 顺序**: `error_map.items()`、`_EXTRACTOR_MAP`、`_case_scoped_value` 的 dict 推导均依赖插入序;Rust 用 `IndexMap` 或 BTreeMap 时注意语义差异(插入序必须用 IndexMap)。
5. **Python repr `!r`**: `_normalized_trace_attribution` 的 `critical_mistake` 使用 `{command[:240]!r}`;若需逐字节一致需手写 Python-repr(单引号、`\\`/`\'`/控制字符转义);若可接受差异,`{:?}` 亦可。
6. **布尔/相等语义**: `check.get("passed") is True` 是身份比较(字符串 `"true"` 不算);`reward == 0 or reward == 0.0` 对 None/字符串为 False;`exit_code in {None, 0, "0"}` 的集合成员语义。
7. **`or` 回退链**: `data.get("x") or 0.0` 会把 0/""/None 都归一为默认;`str(x or "")` 同样。逐处核对。
8. **截断差异**: `_excerpt` 用 `...` 后缀、`_bounded_recent_events` 与 `detail[:2000]` 硬切无后缀、`_truncate_text` 用 `\n...[truncated N chars]` 后缀;三套规则不可混用。
9. **死代码**: `AGGREGATION_*`、`_build_aggregation_prompt`、`_compact_per_case_diagnoses` 当前不参与执行路径;移植时聚合必须走 `_aggregate_structured_diagnoses`。
10. **并发**: `_per_case_diagnosis` 实际串行(配置 `diagnosis_agent_max_concurrency` 未使用);结果顺序 = 输入顺序;若 Rust 端改并发,需保持输出顺序稳定。
11. **YAML 写出**: `safe_dump(allow_unicode=True, sort_keys=False)` 保证中文不转义、键序保持。
12. **JSON 序列化**: 所有 `json.dumps(..., ensure_ascii=False)`;`_build_diagnosis_input_json` 用 `indent=2`;`_write_json` 用 `indent=2`。
13. **`TeamIssue.issue_id` 兜底**: `f"issue_{id(data)}"` 依赖 Python 内存地址,不可移植;Rust 需替换为稳定唯一 id(如递增序号)并接受与 Python 输出的差异(该兜底仅当 LLM 输出缺 issue_id 时触发;确定性聚合路径总是生成 `issue_001` 式 id)。
14. **`datetime.now(UTC).astimezone().isoformat()`**: `created_at` 为本地时区 ISO 8601 字符串(含 UTC 偏移),Rust 用 `chrono` 等价。
15. **`yaml.safe_load` 空文档**: `or {}` 保证返回 dict;Rust `serde_yaml` 需自行处理空/标量顶层。
16. **`str()` 转换**: 多处 `str(value or "")`/`str(item.get(...))` 把 None/数字/列表都字符串化;`str(item)` 对非 str 项也转字符串(如 `_string_list`)。

---

## 8. 附录

### 8.1 `_fingerprint_error` 示例

输入 `"ERROR 2024-01-01T10:00:00Z at /tmp/x.py:12 with 0xDEADBEEF (uuid 550e8400-e29b-41d4-a716-446655440000)"`:
- 时间戳 → `<ts>`;UUID → `<uuid>`;`/tmp/x.py` → `<path>`;`0xDEADBEEF` → `<hex>`;`:12` → `:<N>`。
- 结果: `"ERROR <ts> at <path>:<N> with <hex> (uuid <uuid>)"`。
- 空白归一: 多个连续空白/换行折为单空格。

### 8.2 五个提取器 method_specific 键差异速查

| 提取器 | 注册键 | method_specific 键 | 是否合并 generic.method_specific |
|---|---|---|---|
| Generic | (默认) | `expected_mismatch_cases`, `missing_reference_cases` | —(自身即来源) |
| Pytest | `script_based` | `failed_test_clusters`, `assertion_type_clusters`;任一缺 evidence 时改 `evidence_missing_cases` + `fallback_reason="pytest_evidence_missing"` | 是 |
| Reward | `reward_based` | `zero_reward_cases`, `max_iteration_cases`, `member_harness_issue_cases`, `trajectory_failure_signatures_by_case`, `trajectory_recent_events_by_case`, `normalized_trace_attribution_by_case` | 是 |
| AtomicChecks | `atomic_checks` | `atomic_checks_by_case`, `failed_check_details_by_case`, `partial_score_cases`, `member_harness_issue_cases`, `trajectory_*`, `normalized_trace_attribution_by_case` | **否** |
| LlmJudge | `llm_as_judge` | `low_score_behaviors`, `behavior_score_distribution`, `behavior_diagnostics`, `avg_behavior_score`, `behavior_pass_fail_counts`;(全部缺失时改 `rationale_missing_cases` + `fallback_reason="parsed_dimensions_missing"`;部分缺失时追加 `rationale_missing_cases`) | 是 |

### 8.3 当前"两阶段"的实际执行路径

1. **阶段一(确定性)**: `read_eval_ref` → `read_summary` → `read_case_inputs` → `build_signal_extractor` → `extract`。
2. **阶段二(LLM 逐 case)**: 仅对 `not evaluation_passed` 的 case;每个 case 独立 DeepAgent + JSON 修复循环 + 确定性冲突校验修复。
3. **阶段三(确定性聚合,原设计为 LLM)**: `_aggregate_structured_diagnoses` 按 `(target_ref, failure_mode)` 分组 → 排序 → 取最强项 → `_apply_g5_mapping`。
4. **门面写出**: `per_case_diagnoses.json`(JSON)→ `issues.yaml`(YAML,含 backfill 的 evidence_refs)→ `analysis_ref.yaml`。
