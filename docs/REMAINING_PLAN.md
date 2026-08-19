# 剩余计划(REMAINING PLAN)

> 目标(已修正):**用 Rust 独立实现 agent-core 全部功能,不依赖 Python agent-core 运行时**
> (Python 源码仅在 /Volumes/coder/开源/rs_jiuwen/agent-core 作为规格参考;capability-map 为核对账本)。
> 当前 825 tests / 91 crates,clippy -D warnings 0,fmt clean。
>
> **第三梯队 A(teams / evolving / rsi)已全部完成**(df5584f)。
> **B-1 契约 fixtures(G-02)已落地**:fixtures/ 9 个 seam 的语言中立 golden + ah-app/tests/golden.rs 驱动真实实现验证。
> **B-2 覆盖率门禁已落地**:cargo llvm-cov 实测 workspace 行覆盖率 87.94%,CI 以 --fail-under-lines 80 强制。
>
> **账目更新**(第 111 回合,825 tests / clippy 0 / fmt clean,协调者实现):
> - tools 列表 section ah-plugins-prompt-builder += sections/tools.rs(新模块文件,1:1 对齐 context.py 的 build_tools_content/build_tools_section 纯逻辑):hidden_tools(7 cron_*)+ preferred_order(13)+ 文件组(读写编辑/搜索,全组才用标签行)+ memory_group(5 工具)+ summary_overrides(17 项双语)+ tool_summary(覆盖优先)+ 渲染顺序(首选→组→bash/code→list_skill→记忆组→task_tool→剩余工具首行描述)+ 去重规则 + bash 使用原则(5 条)+ task_tool 使用原则 + 可用代理类型行(复用 context::extract_task_tool_agent_lines)+ build_tools_section(priority=30);7 测试(47 总)
>
> **账目更新**(第 110 回合,818 tests / clippy 0 / fmt clean,协调者实现):
> - context 动态 section ah-plugins-prompt-builder += sections/context.rs(新模块文件,1:1 对齐 harness/prompts/sections/context.py 纯逻辑部分):CONTEXT_HEADER/CONTEXT_FILE_TITLES/CONTEXT_FILES/CONTEXT_SECTION_BY_FILE/DAILY_MEMORY_GUIDANCE 常量 + is_unfilled_template(长度>500 非模板/HTML 注释剥离/6 个模板标记/标题行剥离)+ clean_agent_name(去 (…权威…)/(见 IDENTITY.md) 括号后缀 + 引号标点)+ identity_has_filled_name(名字/Name 行扫描,空或 _() 占位排除)+ build_context_content(头部+单文件标题/内容+空文件提示+每日记忆引导+extra)+ build_context_section(priority=80)+ build_context_file_sections(context.* 单文件)+ extract_task_tool_agent_lines(标记/停止标记切分,行归一化 - 前缀);文件读取与缓存(sys_operation)留待集成;7 测试(40 总)
> - 注:发现 Rust 1.92 工具链移除隐式字符串字面量拼接,多行相邻字面量必须改用 concat! 宏
>
> **账目更新**(第 109 回合,811 tests / clippy 0 / fmt clean,协调者实现):
> - workspace 动态 section ah-plugins-prompt-builder += sections/workspace.rs(新模块文件,1:1 对齐 harness/prompts/sections/workspace.py + workspace_header.py):WORKSPACE_HEADER/IMPORTANT_FILES 双语常量 + DIRECTORY_DESCRIPTIONS 12 项双语描述(get_directory_description 未知→空)+ DirNode(name/path/description/is_file/children)+ format_tree(├──/└──/│ 连接符,目录带 # 描述)+ build_workspace_content(头部 + 路径声明 + 重要文件表)+ build_workspace_section(name=workspace,priority=70);真实目录扫描(sys_operation.fs)留待集成;5 测试(33 总)
>
> **账目更新**(第 108 回合,806 tests / clippy 0 / fmt clean,协调者实现):
> - 技能创建信号 ah-plugins-evolving += skill_creation.rs(新模块文件,1:1 对齐 signal/skill_creation.py):SKILL_CREATION_SIGNAL_PROMPT_ELIGIBLE/SKILL_TOOL_COVER + 阈值常量(首次 6 迭代/10 调用,再次 2 迭代/4 调用)+ normalize_tool_name(命名空间取末段)+ is_effective_task_tool(排除 12 工具名/5 关键词)+ iter_tool_calls/tool_call_name(function.name 优先)/tool_call_id + collect_metrics(watermark 窗口:skill_tool 覆盖标志/有效调用计数/迭代计数(带 tool_call_id 关联回退))+ SkillCreationSignalDetector(skill_tool_used → cover;无快照 → first_prompt_threshold;有快照 → reprompt_threshold 增量)+ SkillStepView(Tool/Llm);8 测试(86 总)
>
> **账目更新**(第 107 回合,798 tests / clippy 0 / fmt clean,协调者实现):
> - 团队信号 ah-plugins-evolving += team_signal.rs(新模块文件,1:1 对齐 signal/team.py):TeamSignalType(user_intent/user_request/trajectory_issue)+ UserIntent/TrajectoryIssue(severity 默认 medium)+ TeamStepView(Tool/Llm 步骤视图)+ build_team_trajectory_summary(关键工具 spawn_member/create_task/build_team/view_task/send_message 更长预算(500/500 vs 150/200)+ tool 20000/llm 10000 字符预算截断带标记)+ make_team_user_intent_signal(source=explicit_request)+ make_team_trajectory_signal(context 携带 trajectory_issues/skill_content,source=passive_trajectory)+ get_team_trajectory_issues(仅 object 项)/get_team_signal_skill_content;7 测试(78 总)
>
> **账目更新**(第 106 回合,791 tests / clippy 0 / fmt clean,协调者实现):
> - 对话信号检测 ah-plugins-evolving += from_conv.rs(新模块文件,1:1 对齐 signal/from_conv.py 确定性部分):失败关键词手写扫描(error 排除后随 = None)+ 用户纠正模式手写展开(中文短语/应该(是用改换)/重新(来做执行尝试)/that's wrong/should be/actually,/no, wait/correct:/fix:)+ skill_md_name(name/SKILL.md 路径提取)+ tool_schema 模式 + ConversationSignalDetector(技能读取历史 → 活跃技能/待定脚本 → script_artifact(代码>20 字符,失败时跳过)/执行失败(数据获取工具跳过,摘录前后 300 字符)/指纹去重 + 默认信号类型过滤 + 无 LLM 用户反馈 fallback);8 测试(71 总)
>
> **账目更新**(第 105 回合,783 tests / clippy 0 / fmt clean,协调者实现):
> - 进化信号 ah-plugins-evolving += signal.rs(新模块文件,1:1 对齐 signal/base.py + from_eval.py):EvolutionCategory/EvolutionTarget 枚举 + EvolutionSignal(signal_type/section/excerpt/skill_name/context + to_dict(context 省略))+ make_evolution_signal(source/tool_name setdefault 归一化)+ get_signal_source + make_signal_fingerprint(signal_type/tool_name/skill_name/excerpt 前 200)+ from_evaluated_case(score>=threshold 过滤,score==0 → low_score 否则 evaluated,excerpt "score=X.XX",source=offline_evaluation)+ from_evaluated_cases(批量);7 测试(63 总)
>
> **账目更新**(第 104 回合,776 tests / clippy 0 / fmt clean,协调者实现):
> - 数据集类型 ah-plugins-evolving += dataset.rs(新模块文件,1:1 对齐 agent_evolving/dataset 包):Case(inputs/label/tools/case_id 自动生成)+ ToolInfo(type 默认 function/name/description/parameters)+ EvaluatedCase(score 夹取 [0,1] + inputs/label/tools/case_id 访问器)+ clamp_score + shuffle_cases(seed 确定性 SplitMix64 Fisher-Yates,原列表不变)+ split_cases(ratio ∈ [0,1] 否则 Err "ratio must be in [0.0, 1.0], got X")+ CaseLoader(len/iter/get_cases/split(洗牌切分,seed 可复现));8 测试(56 总)
>
> **账目更新**(第 103 回合,768 tests / clippy 0 / fmt clean,协调者实现):
> - 进化工具元数据 ah-plugins-evolving += tool_metadata.rs(新模块文件,1:1 对齐 prompts/tools/evolution.py):build_evolution_subject_schema(kind enum skill/swarm-skill,required kind+name)+ 6 个 canonical 工具元数据(prepare_skill_evolution/evolve_review_task/list_skill_experiences(target/section enum 来自 protocols 值集,limit=20,sort 枚举)/read_skill_experiences(max_content_chars=2000)/evolve_skill_experiences/simplify_skill_experiences(actions item 枚举 DELETE/MERGE/REFINE/KEEP))+ 注册表查找(get_evolution_tool_description/get_evolution_tool_input_params,未知工具 Err "not registered. Available: ...")+ build_evolution_tool_card(tool_id_agent_id 或 tool_id_<hex>,ToolCard 结构);8 测试(48 总)
>
> **账目更新**(第 102 回合,760 tests / clippy 0 / fmt clean,协调者实现):
> - 技能沉淀提示 ah-plugins-evolving += skill_creation_sections.rs(新模块文件,1:1 对齐 prompts/sections/skill_creation.py,模板经脚本从 Python 源码精确提取):SKILL_CREATION_GUIDANCE(中/英,技能沉淀自检)+ TEAM_SKILL_CREATION_GUIDANCE(中/英,团队技能沉淀自检)+ TEAM_SKILL_CREATION_NUDGE(中/英,{skills_dir} 占位)+ build_skill_creation_guidance_section/build_team_skill_creation_guidance_section(双语言,priority=88)+ build_team_skill_creation_nudge_section(skills_dir 替换,priority=89);4 测试(40 总)
>
> **账目更新**(第 101 回合,756 tests / clippy 0 / fmt clean,协调者实现):
> - 进化协议提示 ah-plugins-evolving += prompts_sections.rs(新模块文件,1:1 对齐 prompts/sections/evolution.py,模板经脚本从 Python 源码精确提取):EVOLUTION_PROTOCOL_PROMPT(中/英,技能演进自检:判断场景/用户意图信号/回复与确认规则/工具执行)+ TEAM_EVOLUTION_PROTOCOL_PROMPT(中/英,团队 Skill 演进自检)+ pick_prompt(缺失回退 cn)+ build_evolution_protocol_section(name=evolution_protocol,priority=86)/build_team_evolution_protocol_section(name=evolution_team_protocol,priority=87)(复用 ah-contracts PromptSection 契约);5 测试(36 总)
>
> **账目更新**(第 100 回合,751 tests / clippy 0 / fmt clean,协调者实现):
> - 超参常量 ah-plugins-evolving += constant.rs(新模块文件,1:1 对齐 agent_evolving/constant.py):TuneConstant 默认值(default_example_num=1/default_iteration_num=3/default_max_sampled_example_num=10/default_parallel_num=1/default_max_num_sample_error_cases=10/default_early_stop_score=1.0)+ 合法边界(min/max_iteration_num=1/20、min/max_parallel_num=1/20、min/max_example_num=0/20)+ 校验函数(validate_num_parallel(对齐 evaluator batch_evaluate)/validate_num_iterations/validate_example_num(允许 0,对齐 example_optimizer),错误消息 "X should be between A and B");5 测试(31 总)
>
> **账目更新**(第 99 回合,746 tests / clippy 0 / fmt clean,协调者实现):
> - 进化协议字面量 ah-plugins-evolving += protocols.rs(新模块文件,1:1 对齐 agent_evolving/protocols.py):动作(approve/reject/retry)+ 模式(append/merge/replace)+ 效果(state/pending_change)+ 目标/条目(experiences/experience_entry/skill_experience_entry/local_apply_completed)+ 信号(conversation_review/execution_failure/tool_failure/trajectory_issue/user_intent)+ 值集(EVOLUTION_TARGET_VALUES(description/body/script)/EVOLUTION_SUBJECT_KIND_VALUES(skill/team-skill/swarm-skill)/SIMPLIFY_ACTION_VALUES(DELETE/MERGE/REFINE/KEEP)/VALID_PATCH_ACTIONS(append/merge/replace/skip)/VALID_SECTIONS(8 章节))+ 5 个值集校验函数;6 测试(26 总)
>
> **账目更新**(第 98 回合,740 tests / clippy 0 / fmt clean,协调者实现):
> - 更新执行 ah-plugins-evolving += updates.rs(新模块文件,1:1 对齐 agent_evolving/update_execution.py)+ evolving 契约扩展(UpdateMode(Replace/Append/Merge)/UpdateEffect(State/PendingChange)/UpdateValue(new/normalize:experiences → append+pending_change+skill_experience_entry)/ApplyResult(ok())/UpdateKey);normalize_updates(过滤 None)+ execute_updates(operator 缺失 → errors["operator not found: X"],None 值 → errors["update value is None"])+ summarize_apply_results(total/applied/failed);4 测试(20 总)
>
> **账目更新**(第 97 回合,736 tests / clippy 0 / fmt clean,协调者实现):
> - 自进化工具函数 ah-plugins-evolving += utils.rs(新模块文件,1:1 对齐 agent_evolving/utils.py):SkillReferenceScore(ranking_key tool>path>legacy)+ extract_skill_tool_name(payload dict/JSON 字符串)+ find_skill_tool_mentions(手写扫描 skill_tool(skill_name=...))+ infer_skill_from_texts(三源命中统计最优)+ scan_skill_path/scan_legacy_skill_md(手写路径扫描)+ parse_top_level_frontmatter(顶层标量,跳过缩进/list)+ validate_digital_parameter(数值边界)+ convert_dict_to_string(k:v |)+ parse_json_from_llm_response(围栏/原始)+ parse_list_from_llm_response(list 块);8 测试
>
> **账目更新**(第 96 回合,728 tests / clippy 0 / fmt clean,协调者实现):
> - rsi 配置补齐 + 模块拆分 ah-plugins-rsi-config += EvaluatorConfig/DatasetGeneratorConfig/EvaluationResultAnalyzerConfig/TeamSkillOptimizerConfig/OptimizationExperienceLearnerConfig/MemberOptimizerConfig(20 字段)/AutoCoordinatingHarnessConfig(顶层聚合 15 子配置);代码按职责拆分 5 个文件(lib.rs 声明 + core.rs 基础配置 + scheduling.rs 调度/插件 + extended.rs 扩展配置 + auto.rs 顶层聚合);5 测试(11 总)
>
> **账目更新**(第 95 回合,723 tests / clippy 0 / fmt clean,协调者实现):
> - rsi 配置模型 ah-plugins-rsi-config(新 crate,1:1 对齐 rsi/config/config.py):rsi_config 契约(parse_int/parse_float/parse_bool/parse_string_list — bool 拒绝/识别值集/标量→单元素列表)+ DataLoaderConfig(file_pattern/batch_size/batch_balance_keys)+ DatasetCurationConfig(阈值/文件名/来源标签)+ ModelConfigs(7 个模型引用)+ SeedEvaluationConfig(阈值/max_cases)+ OrchestratorSchedulingConfig(固定策略校验 hybrid/team_first_single_pass/epoch_full_evaluation);6 测试;其余 7 个配置类留待后续
>
> **账目更新**(第 94 回合,717 tests / clippy 0 / fmt clean,协调者实现):
> - 数据集加载 ah-plugins-data-loader += parse_json_cases(对齐 loader._load_json_cases:单 case 对象/case 列表/cases 键,非法形状报错)+ batch_plan_payload(对齐 plan_store.write_batch_plan payload:plan_id/dataset_dir/strategy/seed/batch_size/balance_keys/profile_summary/batches(plan entries)/warnings/metadata,无文件 IO);2 测试
>
> **账目更新**(第 93 回合,715 tests / clippy 0 / fmt clean,协调者实现):
> - 数据集画像 ah-plugins-data-loader += DatasetProfiler(1:1 对齐 rsi/data_loader/profiler.py):balance_keys 计数(排除 unknown,按键排序)+ 缺失字段警告(case_id + missing_fields)+ quality 分级(empty/normal/partial_metadata/low_quality_fallback,≥50% 缺失 → low_quality);case_id 完整版(case_id/id 优先,空 → path 文件名 + case_index);5 测试
>
> **账目更新**(第 92 回合,710 tests / clippy 0 / fmt clean,协调者实现):
> - 分批规划 ah-plugins-data-loader(新 crate,1:1 对齐 rsi/data_loader/batch_planner.py + profiler.py):case_value(顶层/metadata 嵌套读取,空→unknown)/case_id(case_id 或 id)+ BatchPlanner::plan(难度升序(easy/medium/hard)+ 组内维度轮转 + batch_size 分组)+ batch_plan_item(batch_id/cases 摘要(难度/维度/来源/类型/路径/索引)/difficulty_stage(主导难度)/dimensions);data_loader 契约(BatchPlanEntry/Case/Metadata);5 测试
>
> **账目更新**(第 91 回合,705 tests / clippy 0 / fmt clean,协调者实现):
> - coding 记忆路径校验 ah-plugins-memory-lite += validate_coding_memory_path(1:1 对齐 coding_memory_tool_ops.py):目录遍历防护 + .md 后缀强制 + coding_memory 目录解析;2 测试
>
> **账目更新**(第 90 回合,703 tests / clippy 0 / fmt clean,协调者实现):
> - 记忆路径校验 ah-plugins-memory-lite += validate_memory_path(1:1 对齐 memory_tool_ops.py):目录遍历防护(.. 与绝对路径拒绝)+ basename 分类解析(USER.md → memory/USER.md;MEMORY.md → memory/MEMORY.md;YYYY-MM-DD.md → memory/daily_memory/;其他 → memory/)+ memory_dir 缺失报错;2 测试
>
> **账目更新**(第 89 回合,701 tests / clippy 0 / fmt clean,协调者实现):
> - 工作区目录模型 ah-contracts += workspace 目录契约(1:1 对齐 harness/workspace/workspace.py):WorkspaceNode 枚举(15 节点:AGENT.md/SOUL.md/HEARTBEAT.md/IDENTITY.md/USER.md/memory/coding_memory/todo/messages/skills/agents/MEMORY.md/daily_memory/.team/.worktree)+ DirectoryNode(name/path/description/is_file/children)+ default_workspace_schema(核心 11 节点含 memory/coding_memory 子节点)+ find_directory_path(递归查找)+ resolve_directory(显式优先 + 默认 schema 回退);4 测试;为 validate_memory_path 与 workspace/context section 提供前提
>
> **账目更新**(第 88 回合,697 tests / clippy 0 / fmt clean,协调者实现):
> - 记忆管理器纯函数 ah-plugins-memory-lite += vector_to_blob/blob_to_vector(f32 LE blob 往返)+ is_recent_session_file(YYYY-MM-DD.md 今天/昨天,北京时区,days_from_civil 历法计算,today 注入可测)+ merge_hybrid_results(向量+关键词加权融合重排降序);3 测试
>
> **账目更新**(第 87 回合,694 tests / clippy 0 / fmt clean,协调者实现):
> - 记忆行视图 ah-plugins-memory-lite += line_range_to_fs_read + view_lines(1:1 对齐 memory_tool_ops.py 的 _line_range_to_fs_read/_view_lines):offset/limit → 1-based line_range(-1 读到 EOF);行切片视图(1-based first_line,line_cap 截断,返回 excerpt/total/start/end/truncated);2 测试
>
> **账目更新**(第 86 回合,692 tests / clippy 0 / fmt clean,3 子代理 + 协调者接手):
> - 工具元数据 ah-plugins-tools-metadata(新 crate,1:1 对齐 harness/prompts/tools/*.py 结构):27 个内置工具的双语描述 + 参数 schema(basic:bash/code/read_file/write_file/edit_file/glob/list_files/grep/powershell/free_search/paid_search/fetch_webpage/search_tools;memory_tools:memory/coding_memory/compression_recall/session_tools/todo/task_tool/goal;special:cron/ask_user/agent_mode/mcp/skill_tool/list_skill/load_tools/lsp_tool),全部经 validate_provider 双语完整性校验;20 测试;描述为与 Python 结构对齐的摘要版,逐字完整素材留待后续
>
> **账目更新**(第 85 回合,672 tests / clippy 0 / fmt clean,协调者实现):
> - 工具元数据契约 ah-contracts += ToolMetadata + validate_provider(1:1 对齐 harness/prompts/tools/base.py):ToolMetadata(name/description_cn/en/params_cn/en/idempotent);validate_provider 双语完整性校验(description 非空 + schema object(properties/required)+ properties key 集合一致 + 递归嵌套 properties/items description 校验);7 测试;工具描述素材生成与 provider 实现留待后续
>
> **账目更新**(第 84 回合,665 tests / clippy 0 / fmt clean,协调者实现):
> - 记忆配置 ah-plugins-memory-lite += MemorySettings(1:1 对齐 core/memory/lite/config.py):model/sources/extra_paths + chunking(256/32)/query(max_results 10/min_score 0.3/hybrid 0.7/0.3/2.0)/store(memory.db,vector+fts)/sync(watch 2000ms/onSearch/onSessionStart)/cache(10000)全量默认 + with_overrides(未知键忽略)+ is_memory_enabled(MEMORY_ENABLED env,默认 true);2 测试
>
> **账目更新**(第 83 回合,663 tests / clippy 0 / fmt clean,协调者实现):
> - 轻量记忆原语 ah-plugins-memory-lite(新 crate,1:1 对齐 core/memory/lite/{frontmatter,types,conflict_types}.py):frontmatter 解析(--- 块 key: value,无 frontmatter/缺闭合返回 None)/校验(name/description/type 必填 + type ∈ user/feedback/project/reference)/丰富(created_at 首次 + updated_at 每次,today 注入可测)/重建(保留正文)/正文提取;MemoryChunk + WriteMode(Create/Append/Skip)+ WriteResult(to_dict 仅含非默认字段);memory_lite 契约;8 测试;coding memory 工具操作(manager/memory_tool_ops)与外部 provider 留待后续
>
> **账目更新**(第 82 回合,655 tests / clippy 0 / fmt clean,协调者实现):
> - 资源标签管理 ah-plugins-tag-manager(新 crate,1:1 对齐 core/runner/resources_manager/tag_manager.py):TagMgr 双向索引(resource_tags + tag_to_resource,含 GLOBAL 预置)+ GLOBAL 语义(打 GLOBAL 替换旧标签、global 资源拒绝其他标签)+ tag_resource/remove_resource/remove_resource_tags(skip_if_not_exists)/update_resource_tags(REPLACE/MERGE)/remove_tag/get_tag_resources/find_resources_by_tags(ANY/ALL,缺失 tag 报错或跳过)/has_resource_tag/get_resources_tags/stats/display;tag_manager 契约(Tag/GLOBAL/TagMatchStrategy/TagUpdateStrategy/TagError);9 测试;agent/tool/model/sys_operation/workflow 资源管理器与取消留待后续
>
> **账目更新**(第 81 回合,646 tests / clippy 0 / fmt clean,协调者实现):
> - stream 管道增强 ah-plugins-stream:stream_output_with_timeouts(首帧超时 + 后续帧超时,对齐 stream_output 的 first_frame_timeout/timeout);2 测试;异步迭代器形态与敏感模式日志留待后续
>
> **账目更新**(第 80 回合,644 tests / clippy 0 / fmt clean,协调者实现):
> - 会话流管道 ah-plugins-stream(新 crate,1:1 对齐 core/session/stream/{emitter,manager,writer}.py):AsyncStreamQueue(tokio mpsc 有界队列,发送超时重试 5 次/接收超时/关闭排空 + 强制清空,stats 统计)+ StreamEmitter(END_FRAME 哨兵,closed 后 emit 报错)+ StreamWriterManager(默认 output/trace/custom writer + stream_output 消费直到 END_FRAME)+ StreamWriter(OutputSchema/TraceSchema/CustomSchema 校验后发射);stream 契约(StreamMode/OutputSchema/TraceSchema/CustomSchema/TeamOutputSchema + StreamError);5 测试;StreamWriterManager 异步迭代器/首帧超时/敏感模式日志留待后续
>
> **账目更新**(第 79 回合,639 tests / clippy 0 / fmt clean,协调者实现):
> - session stream schema ah-contracts += stream 模块(1:1 对齐 core/session/stream/base.py):StreamMode(output/trace/custom + as_str)、OutputSchema(type/index/payload)、TraceSchema(type/payload)、CustomSchema(type/payload)、TeamOutputSchema(OutputSchema + source_member/role,from_output 不修改原 schema);5 测试;StreamEmitter/StreamWriterManager/StreamWriter 状态性管道留待后续
>
> **账目更新**(第 78 回合,634 tests / clippy 0 / fmt clean,脚本自动生成 + 协调者实现):
> - 系统提示 sections ah-plugins-prompt-builder += sections 模块(18 个 build_*_section,双语常量从 harness/prompts/sections/*.py 逐字对齐,priority 一致):基础(identity 10/safety 20/skills 40/todo/task_tool/session_tools)、运行时(heartbeat/memory/coding_memory/prompt_attachments/offload/reload/compression_recall)、高级(agent_mode PLAN_MODE_PROMPT/goal/external_memory(参数化 prompt_block,空→None)/task_completion/progressive_tool_rules);SectionName 27 常量契约;8 测试;workspace(目录扫描)/context(配置文件读取)动态 section 留待后续
>
> **账目更新**(第 77 回合,606 tests / clippy 0 / fmt clean,3 任务并行子代理 + 协调者接手 builder):
> - 系统提示构建器 ah-plugins-prompt-builder(新 crate,1:1 对齐 core/single_agent/prompts/builder.py + harness/prompts/{builder,sanitize,report}.py):prompt_builder 契约(PromptSection 多语言渲染/char_count + PromptMode full/minimal/none + SectionInfo/PromptReport);builder 模块(注册/替换/移除/查询 + priority 升序排序 join + 空白跳过 + Minimal 集合过滤(identity/safety/skills/tools/runtime/prompt_attachments/memory)+ None 模式仅 identity + build_report 委托);sanitize 模块(sanitize_path/sanitize_user_content — 手写字符过滤移除 <>{}[]\`\$ + 三点以上连续点 + 字面 \\n/\\r 序列,Unicode 标量截断);report 模块(from_builder 字符数/估算 token(cn 2.5/en 4.0 截断除)/分 section 统计 + summary 格式);7+7+6 测试
>
> **账目更新**(第 76 回合,586 tests / clippy 0 / fmt clean,3 任务并行子代理):
> - 压缩推断 ah-plugins-reliability-burst += FrequentCompactionDetector(1:1 对齐 reliability/detectors/compaction.py):BEFORE_MODEL_CALL 消息数显著下降(≥drop_ratio 0.3)推断压缩事件,300s 窗口内 ≥3 次 → Medium(边沿触发,触发后锁存);5 测试
> - 团队乒乓 ah-plugins-reliability-tools += PingPongDetector(1:1 对齐 reliability/detectors/pingpong.py):MESSAGE 信号双向方向反转计数,min_volleys 6→Medium/12→High,第三方消息重置 + 边沿触发,evidence 含排序 pair;5 测试
> - 可靠性框架 ah-plugins-reliability-monitor(新 crate,1:1 对齐 reliability/{monitor,remediation,reporter}.py):reliability_config 契约(全量检测器阈值 + 严重度→动作映射 + 重启强度预算);RemediationPolicy 分层策略(LOW observe/MEDIUM report/HIGH steer+report/CRITICAL steer+escalate);LocalAutoRemediator(强度限流 5 次/60s 可逆本地纠偏 + 消息渲染,VecDeque 自实现窗口);ReliabilityMonitor(detectors 聚合 feed + 策略路由 + panic 容忍 + reset);AnomalyReporter trait + LocalAnomalyReporter(进程内 sink 绑定);8 测试
>
> **账目更新**(第 75 回合,568 tests / clippy 0 / fmt clean,2 任务并行):
> - 错误突发检测 ah-plugins-reliability-burst(1:1 对齐 agent_teams/reliability/{window,tool_error,model_error}.py):SlidingWindowCounter(VecDeque 滑窗 add/count/reset)+ ErrorBurstDetector(2×阈值→High/单阈值→Medium,边沿触发,now 注入)+ ToolErrorRateDetector(窗口 60s/rate 5/consec 3)+ ModelErrorRateDetector(同构);7 测试
> - 工具调用可靠性 ah-plugins-reliability-tools(1:1 对齐 reliability/detectors/{output_length,repeat_tool,pingpong}.py):自实现 sha256(FIPS 180-4,已知向量验证)+ canonical JSON(递归键排序)+ stable_call_hash/stable_result_hash;OutputLengthDetector(text 32000/thinking 16000 触发一次);RepeatToolCallDetector 四层(identical≥30→Critical/≥20→High/alternation≥10→Medium/repeats≥10→Low,边沿触发);9 测试
>
> **账目更新**(第 74 回合,552 tests / clippy 0 / fmt clean,2 任务并行):
> - 任务状态机 ah-plugins-team-task-status(1:1 对齐 schema/status.py TaskStatus):TaskStatus 7 态 + 迁移表(Pending→Planning/InProgress/Blocked/Cancelled;Blocked→Pending/Cancelled;Planning 自环(rework)+InProgress/Pending/Blocked/Cancelled;InProgress→InReview/Completed(reviewer 有无)/rework 回 Pending·Blocked·Cancelled;InReview→Completed/InProgress(verify fail rework)/Pending/Cancelled;Completed·Cancelled 无出边)+ is_terminal;12 测试
> - prompt 附件核心 ah-plugins-prompt-attachment(1:1 对齐 harness/prompts/prompt_attachment_manager.py 纯函数部分):PromptAttachment 数据模型(kind 10 种/priority=100/content_kind=text/plain 默认)+ 自实现 sha256(FIPS 180-4,已知向量/NIST 边界/百万 a 验证)+ content_sha256/hash_rendered/hash_attachment(排除 content_sha256·created_at·updated_at 的 canonical JSON 语义哈希,与 Python 精确向量对拍)+ safe_id_part(非法替换/strip/80 截断/全非法 12 位哈希/空 fallback);21 测试
>
**账目更新**(第 73 回合,519 tests / clippy 0 / fmt clean,2 任务并行):
> - 模型分配器 ah-plugins-model-allocator(1:1 对齐 models/allocator.py):RoundRobin(池序轮转/group_index)/ByModelName(分组组内轮转,counters 用 list 避点号键问题,legacy dict 兼容)/Router(唯一映射,无 hint 首项,空池/重名构造 Err)/IntelliRouter(Router 语义 + provider·deployments 校验 Err)+ build_allocator 工厂 + resolve_member_model(纯位置查找,组缩小回 0)+ state_dict/load_state_dict(digest 不匹配归零);20 测试
> - 团队加入描述符 ah-plugins-team-join-descriptor(1:1 对齐 external/descriptor.py):TeamJoinDescriptor(session/team/member 必填,role/scope/language/dispatch_mode/teammate_mode/db/transport 默认)+ to_json/to_env/from_json/from_env(缺键/坏值显式 Err)+ 枚举 serde snake_case;14 测试
>
**账目更新**(第 72 回合,485 tests / clippy 0 / fmt clean,2 任务并行):
> - 交互语法解析 ah-plugins-interaction-router(1:1 对齐 interaction/router.py):parse_mention(@target body)/保留名校验(user·team_leader·human_agent)/parse_interact_str(# god-view、$name avatar-drive(含 (?=@) 无空格写法)、@member 定向、@all/@* 广播覆盖列名收件人、@m1 @m2 多播 fan-out 保序、无前缀缺省 #、#hashtag 非频道)+ resolve_targets(严格匹配 member_exists,未知 mention 折回单条无定向消息保留原文);16 测试
> - 调度消息组装 ah-plugins-scheduler-render(1:1 对齐 scheduling/render.py):meta_task_start(planning 挑 plan 模板)/meta_review_request/renudge/verified_report(refs={task})/meta_rework(params={max_rounds,feedback},空反馈兜底 "无")/format_fail_feedback(- reviewer: feedback 逐行);MessageMeta 构造复用 team_message 契约;10 测试
>
**账目更新**(第 71 回合,459 tests / clippy 0 / fmt clean,3 任务并行):
> - 团队运行时 i18n ah-plugins-team-i18n(1:1 对齐 i18n.py):Language cn/en + t(key,args) 双语查表 + 手写 {var} 占位渲染(缺失键显式 Err;缺失占位保留原样;多余参数忽略)+ reply_hint_for(user 伪成员强制版/其余通用条件版);11 测试
> - 上下文消息正文 ah-plugins-team-context-text(1:1 对齐 messages.py build_identity_text/build_team_info_text):成员身份(两个名字/私有工作区+用途括号 cn（）·en ()/私有约定块)+ 团队信息(名/展示名/目标 + 共享工作空间 mount+path 分支);全空→None;双语标签;16 测试
> - 外部 CLI 入站渲染 ah-plugins-external-format(1:1 对齐 external/format.py):组合 inbound-render + timefmt seam — render_message(<team-inbound> + reply-hint/hitt-silence note,human agent for=controller,框架模板 body 去 hint)/render_messages(bodies 按 message_id)/render_task_line(assignee→或 marker + 相对时间)/render_task_board(过滤终态、角色化标题、<team-event kind=task-board>);7 测试(dev-deps 真实组合)
>
**账目更新**(第 70 回合,425 tests / clippy 0 / fmt clean,4 任务并行子代理):
> - 入站 XML 渲染 ah-plugins-inbound-render(1:1 对齐 inbound_render.py:F_46/F_72):<team-inbound(从·消息id·type·time·for=controller)/team-event(kind 最前·task_id)/team-context> + <team-note> 嵌套最后子元素(双全才渲染)+ 手写 XML 转义(body 保留引号/属性转义引号)+ 快照判定(task-board 类,只留最新整条剔除);18 测试
> - 时间渲染 ah-plugins-timefmt(1:1 对齐 timefmt.py):相对桶(负数/<10s→刚刚;秒/分/时/天)+ 手写 civil 算法绝对时间(负数 epoch/闰日)+ 时区(注入偏移确定性;默认 date +%z→TZ 解析→显式降级 UTC);11 测试
> - 调度扫描核心 ah-plugins-team-scheduler(1:1 对齐 scheduler.py 纯决策):pick_starts(每成员最早 PENDING(assignee),updated_at 升序同则 task_id 字典序,busy 跳过)+ review_decision(内联投票公式:Pass 结算/Fail 达上限升级否则结算/Undecided 停摆升级否则首次送审)+ (task_id,round) 去重键;13 测试
> - 名册 diff ah-plugins-roster-diff(1:1 对齐 prompts/messages.py):joined/left/changed(仅跟踪 display_name/desc/role)+ format_member_line([human]/[prefix])+ 快照/增量双语正文(空→None);5 测试
>
**账目更新**(第 69 回合,378 tests / clippy 0 / fmt clean):
> - 团队消息两阶段渲染 ah-plugins-team-message(1:1 对齐 openjiuwen/agent_teams/message_template.py,F_63):team-message 契约(TaskView/MemberView 字段白名单投影/MessageMeta{template,refs,params}/ExpandedMessage{body,is_template}/RefUnresolved/TeamMessage trait(5 方法)+ TEAM_MESSAGE key);meta 解析(空/畸形/无 template 键 → None 普通消息)/build_meta(params 字符串化)/fallback_line(带 task_id 与不带);单遍 {{ns.field}} 替换(手写扫描,{{ ns.field }} 空白容忍,替换值永不二次扫描,未知 ns/字段 → <missing:ns.field>);expand(普通透传/模板渲染/引用行缺失降级 fallback);6 测试
>
**账目更新**(第 68 回合,372 tests / clippy 0 / fmt clean):
> - 审查投票判定 ah-plugins-team-verdict(1:1 对齐 openjiuwen/agent_teams/agent/scheduling/verdict.py,调度器决策核心 F_62):team-verdict 契约(Verdict(Pass/Fail/Undecided)+ TeamVerdict trait(quorum/judge)+ TEAM_VERDICT key);纯函数投票数学 — quorum=ceil(threshold×reviewer_count);pass_count≥quorum → PASS;fail_count>reviewer_count−quorum(quorum 不可达,决定性迟票即败)→ FAIL;否则 UNDECIDED;reviewer_count≤0 → UNDECIDED;单评审 2/3 阈值退化为首票即定;非法阈值(非有限/≤0)防御 quorum=0;5 测试
>
**账目更新**(第 67 回合,367 tests / clippy 0 / fmt clean):
> - 团队成员/执行状态机 ah-plugins-team-status(1:1 对齐 openjiuwen/agent_teams/schema/status.py):team-status 契约(MemberStatus 10 态/ExecutionStatus 10 态/TeamStatus trait(member_can_transition/execution_can_transition/member_departed/member_unreachable/member_settled/allowed_member_transitions)+ TEAM_STATUS key);成员迁移表(UNSTARTED→STARTING(CAS 守卫)/STARTING→UNSTARTED(回滚)/READY↔BUSY·PAUSED·STOPPED·SHUTDOWN_REQUESTED·SHUTDOWN·ERROR/PAUSED·STOPPED→READY·RESTARTING/SHUTDOWN_REQUESTED→SHUTDOWN·ERROR/SHUTDOWN→RESTARTING(复活)/ERROR→RESTARTING·READY·…);执行迁移表(IDLE→STARTING→RUNNING→(取消链 CANCEL_REQUESTED→CANCELLING→CANCELLED|完成链 COMPLETING→COMPLETED|FAILED|TIMED_OUT) 全部收敛回 IDLE);状态集合 departed(工作守卫)/unreachable(消息投递)/settled(团队完成检查);6 测试
>
**账目更新**(第 66 回合,361 tests / clippy 0 / fmt clean):
> - 团队对象池 + 并发门禁 ah-plugins-team-pool(1:1 对齐 openjiuwen/agent_teams/runtime/pool.py + gate.py):team-pool 契约(InteractGate(唯一 id/admit 票证/consume_done 外门禁忽略/close_and_drain 阻塞等排空/reset;Mutex+Condvar)/ActiveTeam(team_name+session+state+Arc<InteractGate>)/ActiveTeamInfo 只读快照(gate_closed)/TeamRuntimePool trait(7 方法)+ TEAM_POOL key);MutexTeamPool 以 team_name 为键(get/has_active/add 同名替换/remove/list_team_names/teams_for_session/list_all_info);6 测试(池 CRUD·替换/会话分组/快照/gate admit·关闭拒绝/外门禁票证·reset/close_and_drain 跨线程排空)
>
**账目更新**(第 65 回合,355 tests / clippy 0 / fmt clean):
> - 团队运行派发决策 ah-plugins-team-dispatch(1:1 对齐 openjiuwen/agent_teams/runtime/dispatch.py):team-dispatch 契约(RunActionKind 7 种/RuntimeState(Running·Paused)/PoolEntry(current_session_id+state)/RunAction(kind+require_spec+reason)/TEAM_DB_STATE_* 常量/TeamDispatch trait + TEAM_DISPATCH key);纯函数 7 路 truth table — ①非 DB+在 session:db_state ∈ pending_create/cleaned → CREATE(require_spec,先于不一致检查),否则 REJECT_ORPHANED;②非 DB+池有条目 → REJECT_INCONSISTENT;③非 DB → CREATE(require_spec);④DB+无池:在 session → COLD_RECOVER,否则 NEW_TEAM_IN_SESSION;⑤池条目 session 不匹配 → 不变量违例显式 Err;⑥PAUSED → RESUME_FROM_PAUSE;⑦RUNNING → REJECT_RUNNING;8 测试(全表 + 可重建 + 不变量)
>
**账目更新**(第 64 回合,347 tests / clippy 0 / fmt clean):
> - 回放数据集策展 ah-plugins-dataset-curator(1:1 对齐 openjiuwen/rsi/dataset_curator/curator.py):dataset-curator 契约(DatasetCurationConfig(默认 enabled/score_threshold=1.0/require_judgeable_reference/output·report·seed 文件名/source_label)+ DatasetCurationArtifact + DatasetCurator trait + DATASET_CURATOR key);eval_ref 读取(serde_yaml 兼容 JSON);case 决策链(缺失原用例/不确定(status·result.status·evaluation.method==error)/过线(score<阈值,含 result_path 回退)/不可判题(require_judgeable_reference 时)→ 拒绝;否则接受 → replay_{case_id} + metadata(source/synthetic=false/judgeable/provenance(源 id/路径/索引/score/status));定向种子任务(训练信号 → task_pattern/difficulty(easy2·medium3·hard4)/target_capabilities/specific_trap/成功标准(required_behaviors 回退)/failure_summary/轨迹证据(截断 3000)/根因能力(行为得分<0.8,空回退));replay_cases.json + targeted_dataset_seed.json + curation_report.yaml 真实落盘;disabled → disabled 报告;7 测试
>
**账目更新**(第 63 回合,340 tests / clippy 0 / fmt clean):
> - 演化信号映射 ah-plugins-signals(1:1 对齐 openjiuwen/rsi/team_skill_optimizer/signals.py + agent_evolving/signal/base.py):signals 契约(EvolutionSignal(signal_type/section/excerpt/skill_name/context)+ Signals trait(8 方法)+ SIGNALS key);issue_type 归因(attribution.target_ref+category 关键词 → routing_policy/handoff_protocol/shared_context_contract/final_answer_verification/stop_condition,缺省 team_coordination);normalize_trajectory_issue(severity ∈ {low,medium,high} 规整 + type/description/affected_role);issue_description(summary/recommendation + attribution 4 字段拼接,空回退摘录);affected_role(affected_components[0] → evidence(对象/列表) → 空);issue_excerpt(summary→recommendation→description 截断 1000 → 回退 id);issue_ids(1 基编号回退);build_user_query(recommendation 前缀 '- ',空用摘录);build_signals(每条问题一个 trajectory_issue 信号,context 携带 source/trajectory_issues/skill_content/analysis_issue/路径);9 测试
>
**账目更新**(第 62 回合,331 tests / clippy 0 / fmt clean):
> - OpenAI 账号模型目录 ah-plugins-model-catalog(1:1 对齐 openjiuwen/extensions/external_provider/openai_auth/openai_account_models.py):model-catalog 契约(ModelCatalogError(message+status)/ModelCatalog trait(parse_model_ids/add_forward_compat/fetch_models/list_model_ids/read_cache/write_cache)+ MODEL_CATALOG key);实时 GET {base_url}/models(Bearer,ureq;非 2xx 显式带状态码错误);解析 entries 来自 models/data(list 或 dict,dict 注入 slug)、过滤 hide/hidden、id 取 slug/id/name/model 首个非空、按 (priority,id) 排序去重;前向兼容模板(gpt-5.5←gpt-5.4* 等 4 组);兜底链 实时→JSON 缓存(model_ids 或 payload 形式)→内置默认列表;7 测试(含本地 tiny_http /models 服务器)
>
**账目更新**(第 61 回合,324 tests / clippy 0 / fmt clean):
> - LLM 输出 JSON 解析 ah-plugins-json-parser(1:1 对齐 openjiuwen/core/foundation/llm/output_parsers/json_output_parser.py):json-parser 契约(JsonParseError(NoJson/Invalid)+ JsonOutputParser trait(extract_fenced/parse)+ JSON_PARSER key);围栏提取(json 围栏后须换行,Python 正则语义)、跨行提取、内容 trim;parse 优先围栏否则整段(空/纯空白 → NoJson,畸形 → Invalid);StreamJsonParser 流式累加(围栏完整才产出,产出后缓冲区推进到围栏之后,非法 JSON 保留缓冲等待);9 测试
>
**账目更新**(第 60 回合,315 tests / clippy 0 / fmt clean):
> - 经验评分 ah-plugins-experience-scorer(1:1 对齐 openjiuwen/agent_evolving/experience/scorer.py 确定性核心):scoring 契约(UsageStats(used/positive/negative/presented/last_evaluated_at)/ScoredExperience(timestamp ISO/skill_version)/ExperienceScorer trait(5 方法)+ EXPERIENCE_SCORER key);E=贝叶斯平滑 (p+1)/(t+2)(无数据 0.5)/U=used/presented(无数据 0.5)/F=0.5+0.5·2^(-days/90) 指数衰减(版本过期 ×0.7,clamp [0,1])/总分 0.5E+0.3U+0.2F/update 按 used·positive·negative 累加并记录评估时间重算分数;手写 ISO-8601 解析(UTC,支持 Z/±hh:mm 偏移)与 civil↔epoch 转换(无 chrono 依赖)
>
**账目更新**(第 59 回合,307 tests / clippy 0 / fmt clean):
> - 经验分享 ah-plugins-sharing(1:1 对齐 openjiuwen/agent_evolving/sharing):sharing 契约(SharedExperience/SharedSkillBundle(关键词去重保序聚合 + 摘要连接)/SkillPackageMeta/SkillSearchResult/QueryKeywords/UploadResult;SharingBackend 与 ExperienceSharer 两个 async seam + SHARING key);LocalSharingBackend 本地文件 hub(packages/{skill_id}/skill.tar.gz+meta.json、bundles/{skill_id}/{bundle_id}.json、index/{skill_id}.jsonl+global.jsonl,Jaccard 去重阈值拒绝与相关度排序检索,包只保留第一版);ExperienceSharerImpl(stage (skill,record.id) 去重队列/has_pending/discard/flush 打包上传 + 重试退避 + uploaded 镜像 + skill 包同步(provider 解析 skill_id)/download_relevant 下载镜像/list_cached_bundles/search_skills/包下载与元数据)
>
**账目更新**(第 58 回合,298 tests / clippy 0 / fmt clean):
> - telemetry semconv 语义约定(1:1 对齐 Python openjiuwen/extensions/tracer_otel/semconv.py):semconv 模块全量常量(gen_ai.system=openjiuwen / gen_ai.request.model / usage tokens / openjiuwen.workflow.* 9 键 / openjiuwen.agent.* 5 键 / openjiuwen.trace.id·session_id / 基础 span 9 键 / 工作流基础 10 键)+ 四个属性构建助手(gen_ai/agent/workflow/base_span);agent/step 与 tool span 真实携带 openjiuwen.agent.* 与 openjiuwen.status/elapsed_time 等 semconv 属性(原 ad-hoc 键保留兼容)
>
**账目更新**(第 57 回合,293 tests / clippy 0 / fmt clean):
> - 检索重排 ah-plugins-rerank(词法+向量候选按 doc_id+chunk 合并;各路径候选内归一化;lexical_weight × bm25_norm + vector_weight × cosine_norm 加权融合;多样性惩罚(与前 k 个已选共享词扣分);按融合分降序输出 top-k;空输入/负权重显式报错;rerank seam + 注册即反注册)
>
**账目更新**(第 56 回合,289 tests / clippy 0 / fmt clean):
> - RL 训练步数学 ah-plugins-rl-step(advantage=reward−value;policy ratio=exp(Δlogp);clipped PPO objective(ε=0.2,advantage 正负分支);value loss=(v−r)²;policy/value loss 聚合;无效/非有限样本显式剔除)
>
> **账目更新**(第 55 回合,285 tests / clippy 0 / fmt clean):
> - 精确 tokenizer ah-plugins-tokenizer(BPE-lite 字符对合并 + 内置词汇表,CJK 每字一 token,字节偏移;context 引擎注册后 estimate_tokens 用精确计数)
>
> **账目更新**(第 54 回合,281 tests / clippy 0 / fmt clean):
> - 任务记忆服务 ah-plugins-context-evolver(保存 JSONL 持久化/重启恢复;关键词+标签检索打分;轨迹确定性凝练摘要(成功/失败/关键点);检索记忆 + 摘要注入文本可并入 system prompt;Milvus 留待后续)
>
> **账目更新**(第 53 回合,279 tests / clippy 0 / fmt clean):
> - 团队监控 ah-plugins-team-monitor(只读团队/任务/消息视图(经 TeamRuntime)+ 订阅 teams/task 事件记录 seq 单调监控日志;teams 侧 monitor 落地)
>
> **账目更新**(第 52 回合,277 tests / clippy 0 / fmt clean):
> - 成员优化管线 ah-plugins-member-optimizer(attribution:问题→机制(prompt/tool/skill/memory/workflow/context)→lever/目标面;plan:文本梯度;execute:经 Optimizer+OperatorRegistry;verify:val 集得分严格优于基线;publish:best 引用 JSON 落盘)
>
> **账目更新**(第 51 回合,275 tests / clippy 0 / fmt clean):
> - 团队技能生成器 ah-plugins-team-skill(任务 → 确定性计划(关键词→能力词/步骤)+ 注册 skill seam + 在源任务验证(子代理+evolving)+ 修复重试 max_repair_attempts;不合格显式报错)
>
> **账目更新**(第 50 回合,273 tests / clippy 0 / fmt clean):
> - RSI LLM 数据集生成 generate_dataset_llm(提示 → JSON 任务变体解析/去重/截断;LLM seam 可选注入,不可用/响应不可解析时显式回退确定性扩展并注明 source/note)
>
> **账目更新**(第 49 回合,272 tests / clippy 0 / fmt clean):
> - 自进化训练器 ah-plugins-trainer(基线评估 → 每 epoch train 前向 → Optimizer 文本梯度应用 → 验证门禁(严格改进才推进 best)→ early stop;trainer seam)
>
> **账目更新**(第 48 回合,270 tests / clippy 0 / fmt clean):
> - 文本梯度优化器 ah-plugins-optimizer(backward:失败/待改进评估 → 问题→参数路由(工具/记忆/技能/llm)+ 修正指令;step:经 OperatorRegistry.set_parameter 应用,冻结/缺失算子显式记录;与 operator seam 打通)
>
> **账目更新**(第 47 回合,268 tests / clippy 0 / fmt clean):
> - workflow LLM 节点流式消费(run_llm 经 stream_chat:provider 支持 SSE 时真实流式,默认退化单块;content delta 累加 + tool_call delta 按 index 组装)
>
> **账目更新**(第 46 回合,268 tests / clippy 0 / fmt clean):
> - single_harness 迭代编排 + 候选门禁(契约 SingleHarnessRuntime;实现复用 RsiRuntime 评测/精化:train/holdout 拆分,候选在 holdout 得分严格优于 best 才接受,best/checkpoint JSONL 落盘 + 中断续跑)
>
> **账目更新**(第 45 回合,265 tests / clippy 0 / fmt clean):
> - trajectory OTLP span codec(trajectory_codec:Trajectory↔Span 树编解码可逆 + aggregate_trajectories 聚合统计:完成率/错误/平均步数;与 telemetry Span 互通)
>
> **账目更新**(第 44 回合,263 tests / clippy 0 / fmt clean):
> - 知识图谱记忆 ah-plugins-graph-memory(实体抽取:字母数字词 + CJK 二元组确定性;共现实体对 → relation;episode 记录;规范化名合并去重;JSONL 三集合持久化 + 重启恢复;关键词统一打分检索 + 邻居遍历;graph_add_memory/graph_search/graph_neighbors 工具)
>
> **账目更新**(第 43 回合,259 tests / clippy 0 / fmt clean):
> - LLM 流式 seam(ModelChunk/ToolCallDelta + ModelProvider::stream_chat 默认退化单块;OpenAI provider 真实 SSE 解析:data 行逐块,content/tool_calls delta 累加,[DONE] 结束;main.rs 流式演示)
>
> **账目更新**(第 42 回合,257 tests / clippy 0 / fmt clean):
> - Anthropic provider ah-plugins-anthropic(真实 Messages API:/v1/messages,system 顶层拆分,tool_use/tool_result 块,工具 schema→input_schema,x-api-key+anthropic-version 头;本地 HTTP 协议往返测试)
>
> **账目更新**(第 41 回合,252 tests / clippy 0 / fmt clean):
> - runner 回调链 ah-plugins-runner(优先级降序执行 + 链式传参;单回调 retry 上限/timeout 判错/BREAK 短路/ROLLBACK 与错误均逆序执行 rollback handler;CallbackMetrics 调用次数/耗时/错误率)
>
> **账目更新**(第 40 回合,248 tests / clippy 0 / fmt clean):
> - operator 自进化算子 ah-plugins-operator(LLM/tool/memory/skill 四参数句柄:tunables 冻结过滤/set_parameter freeze 检查/on_parameter_updated 回调/检查点 load_state;注册表按 id 取回)
>
> **账目更新**(第 39 回合,244 tests / clippy 0 / fmt clean):
> - controller 控制器 ah-plugins-controller(任务 CRUD/状态机非法迁移拒绝/优先级索引/父子层级防环;执行器注册表 + 同会话 working 冲突拒绝;确定性意图识别 create/pause/resume/cancel/switch 等)
>
> **账目更新**(第 38 回合,241 tests / clippy 0 / fmt clean):
> - PostgreSQL store 后端 ah-plugins-store-pg(真实 SQL:kv/messages 两表 + UPSERT + 增量读 + 幂等建表;GaussDB 兼容,与文件/Redis 同 seam 互换;docker-gated 测试真实连 ah-pg)
>
> **账目更新**(第 37 回合,239 tests / clippy 0 / fmt clean):
> - external CLI agent 运行时 ah-plugins-external(真实子进程:流式长驻 stdin + 单发每轮 argv 两风味;adapter 启动知识/完成标记/steer/abort/超时;TeamJoinDescriptor 编码;codex proto 辅助)
>
> **账目更新**(第 36 回合,236 tests / clippy 0 / fmt clean):
> - Redis 队列后端 ah-plugins-queue-redis(真实外部 provider:LIST 日志 + INCR 序号 + 游标持久,重启续消费;prod 换用,dev 保持本地)
>
> **账目更新**(第 35 回合,235 tests / clippy 0 / fmt clean):
> - cli 渲染(Claude Code 风格:● Tool(args)/⎿ 结果摘要/todo checkbox/⚙ 消息/推理默认隐藏;事件→块投影;ah-cli 实时渲染)
>
> **账目更新**(第 34 回合,230 tests / clippy 0 / fmt clean):
> - symphony 插件(能力注册/指纹/检索/计划/执行,JSONL 持久化 + 工具/subagent 真实执行;已接线 profiles + catalog)
>
> **账目更新**(第 33 回合,228 tests / clippy 0 / fmt clean):
> - agent_builder 790748c(NL→设计→DSL→工作流真实执行)
>
> **账目更新**(第 32 回合,225 tests):
> - 设备码 OAuth 7607726(真实 Device Authorization Grant 流)
>
> **账目更新**(第 31 回合,223 tests):
> - OTLP/JSON 导出 53afa6a(telemetry 缺口)
>
> **账目更新**(第 30 回合,222 tests):
> - 渐进披露 ApprovalRail 09bc486(tool-approval seam + pre-execute rail,批准集持久化)
>
> **账目更新**(第 29 回合,220 tests):
> - tune 训练流水线 409606a(真实 subagent 执行 + evolving 评估/优化 + 最优跟踪)
>
> **账目更新**(第 28 回合,218 tests):
> - graph/Pregel b89bbc6(超级步引擎 + 条件路由 + 中断 + 上限)
>
> **账目更新**(第 27 回合,215 tests):
> - A2A SSE 流式 a273190(text/event-stream 端点 + stream_send 客户端)
>
> **账目更新**(第 26 回合,214 tests):
> - skill 注册 + 评估 dbe36a2(文件后端 + subagent 委派 + evolving 轨迹)
>
> **账目更新**(第 25 回合,212 tests):
> - workflow 检查点续跑 d2bbbc1(JSONL 逐节点落盘 + resume)
>
> **账目更新**(第 24 回合,211 tests):
> - workflow Questioner 节点 af96eb5(经 queue 真实问答 + 超时)
>
> **账目更新**(第 23 回合,209 tests):
> - mcp-http 客户端 0293962(streamable-HTTP 风格 POST JSON-RPC)
>
> **账目更新**(第 22 回合,207 tests):
> - Redis 后端 543ee4b(真实外部 provider,SET/GET/DEL/KEYS,prod 换用)
>
> **账目更新**(第 21 回合,206 tests):
> - 类型化子代理 e824604(code/research/plan/verify + 工具白名单真实强制)
>
> **账目更新**(第 20 回合,204 tests):
> - agent_rl reward dac0c8d(确定性线性奖励函数)
>
> **账目更新**(第 19 回合,201 tests):
> - workflow Http + Intent 节点 6abc2c9(真实 HTTP / 关键字 + LLM 路由)
>
> **账目更新**(第 18 回合,198 tests):
> - evolving experience 持久化 bcf9d87(save/load/search JSONL + 跨重开恢复)
>
> **账目更新**(第 17 回合,196 tests):
> - auto-harness 六阶段编排 021af8d(assess→publish,真实 git 提交 + ci 门禁)
>
> **账目更新**(第 16 回合,193 tests):
> - evaluation_result_analyzer aa892da(确定性信号 + 根因归因 + analysis.json)
>
> **账目更新**(第 15 回合,190 tests):
> - git seam c25e510(真实子进程:init/add/commit/log/status/diff/branch)
> - ci seam ddd1157(子进程门禁 + 超时 + 通过判定)  —— auto_harness git/CI 基建完成
>
> **账目更新**(第 14 回合,182 tests;目标修正为不依赖 Python):
> - swarmflow 真实引擎 14644a5(agent_teams workflow 缺口,替换 MockWorkflowStep)
>
> 下一优先级:auto_harness 真实 git/CI → evaluation_result_analyzer → 外部 provider(docker)→
> evolving 持久化/agent_rl → core workflow/Pregel/controller → harness subagents/cli → dev_tools。
>
> **账目更新**(第 13 回合,173 tests):
> - rsi 多轮编排 5c25592(run_rounds:评测→精化→checkpoint 续跑)
> - web_fetch / run_code 真实工具 f636780(agent 可调用,seam 消费方闭环)
>
> **账目更新**(第 12 回合,169 tests):
> - transport(A2A 风格 JSON-RPC over HTTP)35e0799 —— capability-map **零 missing** 达成
>
> **账目更新**(第 11 回合,166 tests):
> - ah-cli 全 seam 子命令 fac02b3(teams/rsi/workspace/web/queue/code)+ 真实二进制 e2e 冒烟测试
>
> **账目更新**(第 10 回合,165 tests):
> - code 真实执行 cb1f07a(隔离 scratch + python3 子进程 + 超时强杀)
> - web 真实 HTTP 3ed25db(ureq GET + 超时,本地 TCP 真实协议测试)
>
> **账目更新**(第 9 回合,159 tests):
> - teams 消息经 queue c6f929a(send_message/messages,queue seam 消费方闭环)
> - sandbox 策略化沙箱 50bd531(sandbox.json 策略 + pre-execute rail)
>
> **账目更新**(第 8 回合,154 tests):
> - queue 本地基础 1840fc3(日志+游标 offset 语义,重启恢复)
> - workspace 清单与目标 7cfa80f(workspace.json + 目标状态机)
>
> **账目更新**(第 7 回合,150 tests):
> - teams SQLite 持久化 be31244(SqliteTeamRuntime 真实事务,重启恢复)
> - session 检查点 2cea108(checkpoint/restore 命名快照,seq 续接)
>
> **账目更新**(第 6 回合,145 tests):
> - retrieval 向量化 9483740(哈希 n-gram TF embedding + 余弦,CJK 二元组,search 工具 vector 模式)
> - harness rails ebc089c(PathGuard 路径逃逸 + ToolBudget 调用上限,复用 pre-execute 挂载点)
> - prompt seam 三角闭环 6aa8d54(agent-loop 消费方:注册模板渲染注入系统提示)
>
> **账目更新**(第 5 回合,137 tests):
> - context 引擎 8c59fe4(token 预算组装/压缩/offload/reinject)
> - store 本地文件后端 c039873(BaseKVStore + BaseMessageStore)
> - prompt 版本化注册表 f93810e({{var}} 渲染 + 缺失显式报错 + 文件持久化)
> - context seam 三角闭环 1ed1cb8(agent-loop 与 subagent 作为消费方按预算组装)
>
> **历史账目**:
> - teams 95abe79 · evolving a089fd9 · rsi df5584f(第三梯队 A 全部交付验收)
> - G-02 契约 fixtures 7a7d7de(fixtures/ + ah-app/tests/golden.rs)
> - 覆盖率门禁 51da82b(CI coverage job + 实测 87.94%)
> - 子代理日志补齐 AgentStep(随 df5584f)
>
> **下一步建议**:差分契约与跨平台需外部环境,建议委派任务书;域深化按 C 节推进
> (context_engine 优先,其次 store 真实后端 / retrieval 向量化)。
> 执行模式:规划(本表)→ 派发执行对话 → 主对话验收(构建/测试/clippy/真实性/架构)。

## A. 第三梯队大域(按依赖)

| # | 任务 | 状态 | 验收要点 |
| --- | --- | --- | --- |
| 7 | teams(多 agent 任务协作) | ✅ 已验收 | 任务板/依赖/review/settle 真实状态迁移;run_task 真实委派 subagent;成员校验;teams/task 事件 |
| 8 | evolving(轨迹/evaluator/optimizer) | ✅ 已验收 | 真实轨迹抽取(会话日志配对/预算/完成标志);本地判据评估 + LLM judge 附加(不可用显式记录);优化建议真实规则推导 + LLM 附加 |
| 9 | rsi(数据生成/评测/优化编排) | ✅ 已验收 | 端到端 RSI:数据集确定性扩展、用例经 subagent 真实执行 + evolving 评估、报告聚合、提示精化、JSONL checkpoint 续跑 |

## B. 工程收尾

| 项 | 状态 | 说明 |
| --- | --- | --- |
| CI mock 门禁 | ✅ 0f55d52 | prod profile 无 mock 插件,测试+CI 强制 |
| 契约 fixtures(G-02) | ✅ 已落地 | fixtures/ 9 seam golden(fs/session/tools/memory/retrieval/security/evolving/teams/rsi),成功/非法/序列化/恢复类目;超时/取消多为 N/A(本地无超时语义),后续 seam 补充 |
| 覆盖率门禁(≥80%) | ✅ 已落地 | llvm-cov 实测 87.94%;CI coverage job --fail-under-lines 80 |
| 差分契约(与 agent-core 对等) | 机制 + Rust 基线已落地(本回合 references/ 5 seam);Python 参考数据待外部生成 | references/ 完整输出快照 + differential.rs 断言门禁 |
| 跨平台验证 | 待办 | Linux/Windows 目前未跑 |

## C. 域深化(capability-map 缺口)

| 域 | 任务 | 说明 |
| --- | --- | --- |
| core | context_engine ✅ / store 本地文件后端 ✅ / prompt ✅(+agent-loop 消费)/ retrieval 向量化 ✅ / harness rails ✅(本回合)| 下一步:browser/web/lsp 真实工具(需外部)或 teams 深化 |
| harness | 剩余 rails ✅ / workspace-goal-manifest ✅(本回合)/ browser/web/lsp 真实工具 | workspace 清单与目标已落地;browser/web/lsp 需外部进程或网络,留待后续 |
| extensions | store(Redis/GaussDB/ES)/ queue(Pulsar)/ sandbox(远程)/ openai OAuth / a2a / mcp-http | 本地 queue ✅、本地 sandbox 策略 ✅(本回合);外部协议路径需容器/网络,留待后续 |
| teams 深化 | 外部 CLI 进程 / ZMQ / SQLite 持久化 / 消息 | SQLite 持久化 ✅;消息经 queue seam ✅(本回合);外部 CLI 进程 / ZMQ 留待后续 |

## 执行顺序(当前)

7(teams)→ 验收 → 8(evolving)→ 验收 → 9(rsi)→ 验收;并行穿插 B 项;A/C 深化按依赖推进。