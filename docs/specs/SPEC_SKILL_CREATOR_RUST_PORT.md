# Skill Creator 五阶段流水线 —— Rust 1:1 移植规格文档

> 本文档依据以下 6 个 Python 源文件提取,作为 Rust 侧 1:1 移植的权威规格:
>
> | 文件 | 行数 | 作用 |
> |---|---|---|
> | `scripts/common.py` | 303 | 共享工具(路径/slug/JSON/资产/blocks 处理) |
> | `scripts/stage_01_scrape.py` | 510 | 抓取页面 → 统一 `blocks[]` + `video_urls` |
> | `scripts/stage_02_download.py` | 109 | 下载并去重图片块 |
> | `scripts/stage_03_filter.py` | 164 | LLM 视觉过滤图片相关性 |
> | `scripts/stage_04_save.py` | 76 | 保存通过过滤的图片到 `skills/<slug>/references/` |
> | `scripts/stage_05_generate.py` | 102 | LLM 生成 `SKILL.md` + 确定性后处理 |
>
> 源文件所在目录:`agent-core/openjiuwen/dev_tools/skill_creator/skills/skill_omni_creation/scripts/`
> (下文行号均指该目录下的原始文件行号)。

---

## 0. 总览

### 0.1 五阶段流水线

```
URL ──▶ [stage_01 抓取] ──▶ work/<slug>/stage_01_scrape.json
            │ (Playwright 渲染 + DOM 遍历;失败时 LLM web 插件兜底)
            ▼
       [stage_02 下载] ──▶ work/<slug>/stage_02_download.json
            │ (HTTP 下载图片 + SHA-256 内容去重 + 尺寸/大小限制)
            │   资产落盘:work/<slug>/assets/dom_NNN.ext
            ▼
       [stage_03 过滤] ──▶ work/<slug>/stage_03_filter.json
            │ (LLM 视觉批量 KEEP/SKIP,带前后文)
            ▼
       [stage_04 保存] ──▶ work/<slug>/stage_04_save.json
            │   图片复制到 skills/<slug>/references/img_NN.ext
            │   blocks 中 image 块写入 "path" 字段
            ▼
       [stage_05 生成] ──▶ skills/<slug>/SKILL.md
            │ (LLM 生成 + 去幻觉图片引用 + 追加 Reference Files)
            │   --clean 时删除 work/<slug>/
```

每个 stage 都是独立 CLI 进程,通过 JSON 文件传递数据;stage N+1 的输入是 stage N 的输出(最后两个 stage 还读取 `asset_dir` 指向的二进制资产目录)。

### 0.2 目录约定(相对进程 CWD)

| 路径 | 内容 |
|---|---|
| `work/<slug>/stage_01_scrape.json` | stage 1 输出(也是 stage 2 输入) |
| `work/<slug>/stage_02_download.json` | stage 2 输出(也是 stage 3 输入) |
| `work/<slug>/stage_03_filter.json` | stage 3 输出(也是 stage 4 输入) |
| `work/<slug>/stage_04_save.json` | stage 4 输出(也是 stage 5 输入) |
| `work/<slug>/assets/dom_NNN.ext` | stage 2 下载的原始资产(manifest 记录相对路径) |
| `skills/<slug>/references/img_NN.ext` | stage 4 保存的最终图片(供 SKILL.md 引用) |
| `skills/<slug>/SKILL.md` | stage 5 生成的最终产物 |

- `work` 与 `skills` 均为相对路径,相对进程启动时的 CWD(由 `work_path`、`Path(args.skills_dir)` 决定)。
- stage 2/3/4/5 的 JSON 通过 `**data`(解包)逐级携带上游字段,因此 **`url`、`slug`、`title`、`video_urls` 字段会贯通全部阶段**(stage 3 会重写 `title`)。

---

## 1. common.py —— 共享工具规格

### 1.1 模块级常量与导入期行为

| 常量 | 值 | 行号 |
|---|---|---|
| `ROOT` | `Path(__file__).resolve().parents[1]`,即 `skill_omni_creation/` 目录 | L11 |
| `API_BASE` | 环境变量 `API_BASE` | L14 |
| `API_KEY` | 环境变量 `API_KEY` | L15 |
| `MODEL` | 环境变量 `MODEL_NAME` | L16 |
| `STEALTH_UA` | `"Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36"` | L18-22 |
| `SUPPORTED_EXTS` | `{".png", ".jpg", ".jpeg", ".webp", ".gif"}` | L24 |
| `SUPPORTED_MIMES` | `{"image/jpeg", "image/png", "image/gif", "image/webp"}` | L25 |
| `MIME_TO_EXT` | `{"image/jpeg": ".jpg", "image/png": ".png", "image/gif": ".gif", "image/webp": ".webp"}` | L26 |
| `MIN_DIMENSION` | `80` | L27 |
| `MAX_IMAGE_BYTES` | `5 * 1024 * 1024` = 5 MiB | L28 |
| `FETCH_WORKERS` | `10` | L29 |
| `FILTER_BATCH` | `5` | L30 |
| `FILTER_WORKERS` | `3` | L31 |
| `VIDEO_FRAMES` | `4` | L32 |

- **导入期副作用**:`load_dotenv(dotenv_path=ROOT / ".env")` 先加载 `skill_omni_creation/.env`,随后 `API_BASE = os.environ["API_BASE"]` 等三行直接索引环境变量。**三个变量任一缺失 → 导入时抛 `KeyError`(Rust 移植:在进程启动阶段显式校验并报错,消息应指明缺失的变量名,如 `KeyError: 'API_BASE'`)。**
- `VIDEO_FRAMES`(L32)在本流水线的 6 个文件中**从未被引用**,属于死常量;Rust 侧可省略或保留注释说明。

### 1.2 LLM 提示词常量(LLM 驱动,不可 1:1 移植为 Rust 逻辑,但必须原样保留文本)

| 常量 | 行号 | 用途 |
|---|---|---|
| `FILTER_PROMPT` | L36-62 | stage 3 的系统提示词:定义 KEEP/SKIP 判据(保留软件界面截图;跳过小图标/logo/装饰/广告/无关内容;subpage 图片从严;输出 `["KEEP","SKIP",...]` 纯 JSON 数组) |
| `SKILL_PROMPT` | L64-167 | stage 5 的系统提示词:定义 SKILL.md 的完整格式规范(frontmatter、分组规则、图片规则、禁幻觉、Focus 规则等) |
| `SCRAPE_FALLBACK_PROMPT` | L169-191 | stage 1 LLM 兜底抓取提示词:要求返回 `{"title", "blocks", "video_urls"}` JSON;text 块 ≤300 字符 |

**Rust 移植要求**:三个提示词常量以字符串常量原样嵌入 Rust 二进制(逐字节一致,含 `\n` 换行与缩进),并随 LLM 请求原样发送。提示词内容的规则约束(如 SKILL.md 骨架、分组规则)属于"由 LLM 执行的确定性规范",Rust 侧必须把它们作为常量保留,不应尝试用 Rust 代码重新实现提示词内部的规则(除了第 9 节列出的代码级确定性后处理)。

### 1.3 JSON 工具

#### `load_json(path: pathlib.Path) -> dict`(L196-197)
- 语义:`path.read_text(encoding="utf-8")` 后 `json.loads`。
- 错误行为:**不捕获异常**。文件不存在 → `FileNotFoundError`;内容非法 → `json.JSONDecodeError`。两者均向上传播(CLI 进程直接崩溃打印 traceback)。
- Rust 对应:读文件 → `serde_json::from_str`,错误用 `anyhow`/`thiserror` 包裹并带上路径上下文;建议错误消息包含路径,便于排障。

#### `write_json(path: pathlib.Path, data: dict) -> None`(L200-202)
- 语义:`path.parent.mkdir(parents=True, exist_ok=True)` 先建父目录,再以 `json.dumps(data, ensure_ascii=False, indent=2)` 写 UTF-8。
- **关键细节**:`ensure_ascii=False` → 非 ASCII(如中文)按原字符输出,不做 `\uXXXX` 转义;`indent=2` → 2 空格缩进。
- Rust 对应:`serde_json::to_string_pretty`(默认 2 空格缩进)+ 直接输出 Unicode 字符(不要 `escape_non_ascii`)。

#### `strip_json_fence(text: str) -> str`(L205-209)
- 语义(顺序严格):
  1. `text.strip()`;
  2. `re.sub(r"^```[a-z]*\n?", "", text)` —— 去掉开头的 ``` 围栏行(语言标识可选,如 ` ```json`),连同其后至多一个换行;
  3. `re.sub(r"\n?```$", "", text)` —— 去掉结尾的 ``` 围栏行(至多一个前置换行);
  4. 再次 `text.strip()`。
- 纯函数,确定性,可 1:1 移植。

#### `encode_b64(data: bytes, mime: str) -> str`(L212-213)
- 语义:返回 `f"data:{mime};base64,{base64.standard_b64encode(data).decode()}"`,即 **Data URL,标准 base64(含 `=` padding)**。
- Rust 对应:`base64::engine::general_purpose::STANDARD`(默认带 padding)。

### 1.4 路径 / slug 工具

#### `slugify(text: str) -> str`(L218-222)
- 语义(顺序严格):
  1. `text.lower().strip()`;
  2. `re.sub(r"[^\w\s-]", "", text)` —— 删除所有非"单词字符/空白/连字符"字符;
  3. `re.sub(r"[\s_-]+", "_", text)` —— 将空白/下划线/连字符的连续串折叠为单个 `_`;
  4. `text[:80]` —— 截断到 80 字符。
- **移植注意(Python vs Rust 正则)**:Python `re` 的 `\w` 在 str 上按 Unicode 语义匹配(包含 CJK 汉字、全角字符、下划线);Rust `regex` crate 的 `\w` **默认同样是 Unicode 感知**(与 Perl 不同),行为一致,可 1:1。若选用 ASCII-only 的 regex 配置则会产生差异,勿用。
- 纯函数,确定性,可 1:1 移植。

#### `url_to_slug(url: str) -> str`(L225-228)
- 语义:`parsed = urlparse(url)`;`raw = (parsed.netloc + parsed.path).strip("/")` —— 取 **host[:port] + path,去掉 scheme/query/fragment,首尾 `/` 去除**;再 `slugify(raw)`。
- 例:`https://support.microsoft.com/en-us/office/create-a-pivotchart...` → slug 为 `support.microsoft.com_en-us_office_create-a-pivotchart...` 的 slugify 结果。
- **移植注意**:Python `urlparse` 容错性强(畸形 URL 不抛错);Rust `url::Url::parse` 严格,畸形 URL 返回错误。为 1:1 行为,建议 Rust 侧对解析失败采用宽松回退(如直接用字符串切割 `//` 之后、`?`/`#` 之前的部分),或按 Python 语义自行实现宽松解析。
- 纯函数(无 IO),确定性,可 1:1。

#### `work_path(slug: str, filename: str) -> pathlib.Path`(L231-232)
- 语义:返回 `pathlib.Path("work") / slug / filename`,即相对 CWD 的 `work/<slug>/<filename>`。
- 纯函数,可 1:1。

#### `image_ext(url: str, mime: str) -> str`(L235-239)
- 语义:
  1. `ext = pathlib.Path(urlparse(url).path).suffix.lower()` —— URL path 的扩展名(含点、小写);path 无扩展名时为空字符串;
  2. 若 `ext not in SUPPORTED_EXTS`(含空串)→ `ext = MIME_TO_EXT.get(mime, ".png")`(未知 mime 默认 `.png`);
  3. 返回 `ext`(始终带前导点)。
- 纯函数,确定性,可 1:1。

### 1.5 资产工具

#### `save_fetched_assets(fetched: dict[str, tuple[bytes, str]], asset_dir: pathlib.Path, prefix: str) -> dict[str, dict]`(L244-256)
- 参数:`fetched` = `{url: (bytes, mime)}`(**dict 保持插入顺序**);`asset_dir` 输出目录;`prefix` 文件名前缀(本流水线用 `"dom"`)。
- 语义:
  1. `asset_dir.mkdir(parents=True, exist_ok=True)`;
  2. 按 `fetched.items()` 插入顺序枚举,`idx` 从 0 起:`rel_path = Path(f"{prefix}_{idx:03d}{image_ext(url, mime)}")` → 如 `dom_000.png`(**3 位零填充序号**);
  3. 写入 `out_path.write_bytes(data)`;
  4. `manifest[url] = {"path": rel_path.as_posix(), "mime": mime}`(`as_posix()` 把 Windows 反斜杠转 `/`,在 macOS/Linux 下原样)。
- 返回:manifest dict,`key = 原始 URL`,`value = {"path": 相对文件名, "mime": mime}`。
- 错误:文件写失败异常向上传播;目录自动创建。
- **确定性保证**:只要 `fetched` 插入顺序确定,生成的文件名序号即确定(调用方 stage 2 保证按 blocks 顺序插入,见 §4.2)。

#### `load_fetched_assets(asset_dir: pathlib.Path, manifest: dict[str, dict]) -> dict[str, tuple[bytes, str]]`(L259-264)
- 语义:对 manifest 每一项 `path = asset_dir / meta["path"]`,`fetched[url] = (path.read_bytes(), meta["mime"])`。
- 错误:manifest 缺 `path`/`mime` 键 → `KeyError`;文件缺失 → `FileNotFoundError`。均不捕获、向上传播。
- Rust 对应:按 `meta["path"]` 相对路径拼接(防目录穿越:建议校验 `meta["path"]` 不含 `..` 等,Python 侧无此防护,1:1 可不加,但移植时建议加)。

### 1.6 blocks 工具

#### `blocks_with_paths_as_str(blocks: list[dict]) -> list[dict]`(L269-276)
- 对每个 block:若 `type == "image"` 且 `path` 非 `None`,则浅拷贝并 `path: str(b["path"])`;否则原样保留。用于把 Path 序列化为 JSON。

#### `blocks_with_paths_as_path(blocks: list[dict]) -> list[dict]`(L279-286)
- 反向:对 image 块 `path: pathlib.Path(str(b["path"]))`。用于从 JSON 恢复 Path。

#### `strip_hallucinated_images(md: str, valid_paths: set[str]) -> str`(L289-303)
- 语义(确定性后处理,stage 5 使用):
  1. 正则 `!\[([^\]]*)\]\(([^)]+)\)` 匹配所有 markdown 图片引用(alt 可为空,path 至少 1 字符且不含 `)`);
  2. 对每个匹配:`path = match.group(2).strip()`;若 `path in valid_paths` 保留整行原样,否则替换为空串(删除该图片引用);
  3. 逐行重建:**保留所有非空行;仅当上一保留行为非空行时才保留空行** —— 效果 = 折叠连续空行为至多 1 个、丢弃开头空行;最后整体 `.strip()`。
- 纯函数,确定性,可 1:1 移植(注意第 3 步是"折叠空行"而非"删除所有空行")。

---

## 2. 阶段间数据契约(JSON 格式)

### 2.1 stage_01_scrape.json(输出/stage2 输入)

```json
{
  "url": "https://example.com/guide",
  "slug": "example.com_guide",
  "title": "<page title,可能为空串>",
  "blocks": [
    {"type": "heading", "level": 2, "text": "...", "source": "main"},
    {"type": "text",    "text": "...", "source": "main"},
    {"type": "image",   "url": "https://.../a.png", "alt": "...", "source": "subpage", "path": null}
  ],
  "video_urls": ["https://www.youtube.com/watch?v=..."]
}
```

- `blocks` 元素只有三种 `type`:`heading`(必带 `level` 1-4)、`text`、`image`。所有块必带 `source`(`"main"` 或 `"subpage"`);`image` 块必带 `path` 字段,初始为 `null`,后续 stage 填充。
- `video_urls`:字符串数组,保持首次出现顺序去重。

### 2.2 stage_02_download.json(输出/stage3 输入)

在 stage_01 的全部字段之上,**`image` 块中下载失败的会被删除**,并新增:

```json
{
  "...stage_01 字段...",
  "fetched_assets": {
    "https://.../a.png": {"path": "dom_000.png", "mime": "image/png"}
  },
  "asset_dir": "work/<slug>/assets"
}
```

- `fetched_assets`:key = 原始 URL;`path` = 相对 `asset_dir` 的文件名。
- `asset_dir`:posix 风格相对路径字符串。

### 2.3 stage_03_filter.json(输出/stage4 输入)

在 stage_02 字段之上,`title` 被重写为 `args.title or data.get("title")`(必非空),`blocks` 中被 LLM 判 SKIP 的 image 块被删除。`fetched_assets`/`asset_dir` 原样保留。

### 2.4 stage_04_save.json(输出/stage5 输入)

```json
{
  "...stage_03 字段...",
  "slug": "<slug>",
  "skills_dir": "skills",
  "skill_dir": "skills/<slug>",
  "blocks": [
    {"type": "image", "url": "...", "alt": "...", "source": "main",
     "path": "skills/<slug>/references/img_00.png"}
  ]
}
```

- **image 块的 `path` 被填充为相对 CWD 的路径字符串**(通过 `blocks_with_paths_as_str`,即 `skills/<slug>/references/img_NN.ext`)。
- `skills_dir` 存的是 CLI 参数原样字符串(默认 `"skills"`);`skill_dir` 是 posix 字符串 `skills/<slug>`。
- `fetched_assets`/`asset_dir` 原样保留(stage 5 不再读取,但数据仍在)。

### 2.5 最终产物:skills/<slug>/SKILL.md

见 §9.3 骨架;另参考文件 `skills/<slug>/references/img_NN.ext`。

---

## 3. stage_01_scrape.py 规格(抓取)

### 3.1 常量与黑名单(确定性)

| 常量 | 值 | 行号 |
|---|---|---|
| `UTILITY_PATHS` | `/login /signin /signup /register /logout /privacy /terms /tos /cookies /legal /about /contact /faq /help /support /careers /cart /checkout /payment /subscribe /search /sitemap /404 /403` | L24-30 |
| `AD_DOMAINS` | `doubleclick.net, googlesyndication.com, googleadservices.com, googletagmanager.com, google-analytics.com, adnxs.com, criteo.com, criteo.net, outbrain.com, taboola.com, moatads.com, rubiconproject.com, pubmatic.com, openx.net, scorecardresearch.com, quantserve.com, hotjar.com, facebook.com, connect.facebook.net, cookielaw.org, onetrust.com` | L32-40 |
| `AD_PATH_KEYWORDS` | `/ads/ /ad/ /banner/ /banners/ /tracking/ /pixel/ /beacon/ /analytics/ /telemetry/ /sponsored/ /promo/`(均为子串匹配) | L42-47 |
| `PLATFORM_PATTERNS` | `youtube\.com/watch`、`youtu\.be/`、`bilibili\.com/video`、`vimeo\.com/\d+`、`twitter\.com/.+/status`、`x\.com/.+/status` | L49-53 |
| `COOKIE_SELECTORS` | 11 个 Playwright CSS 选择器(OneTrust accept 按钮、`accept-all`、`Accept all`、`Accept cookies`、`I agree`、`Agree` 等) | L55-67 |
| `NOISE_IDS` | `onetrust-consent-sdk, onetrust-banner-sdk, onetrust-pc-sdk, cookie-law-info-bar, gdpr-cookie-notice, CybotCookiebotDialog` | L69-72 |
| `NOISE_TABPANEL_LABELS` | `discover, community, contact us, windows insiders, related resources, more resources`(小写比较) | L74-77 |
| `NOISE_SUBPAGE_PATHS` | `/accessibility /security /rss /windows-insiders` | L79-81 |
| `NOISE_CLASSES` | `uhf, c-uhfh, c-footer, c-nav, breadcrumb, feedback, social, c-heading-4, ocr` | L83-86 |

### 3.2 URL 判定函数(纯函数,可 1:1)

#### `is_utility_url(url: str) -> bool`(L91-96)
- `path = urlparse(url).path.lower().rstrip("/")`;返回 True 当且仅当存在 `p ∈ UTILITY_PATHS` 使 `path == p` 或 `path.startswith(p + "/")`。
- 例:`/login`、`/login/`、`/login?x=1`(query 不在 path 内)均命中;`/logins` 不命中。
- **任何异常 → 返回 False**(含 urlparse 失败)。

#### `is_ad_url(url: str) -> bool`(L99-110)
- 规则 1(域名后缀匹配):`host = netloc.lower().lstrip("www.")`;`parts = host.split(".")`;对 `i in range(len(parts) - 1)`(**不含最后一个单标签**),若 `".".join(parts[i:]) in AD_DOMAINS` → True。即:AD_DOMAINS 本身或其任意子域均命中;`www.` 前缀被剥离。
- 规则 2(路径子串):`any(kw in parsed.path.lower() for kw in AD_PATH_KEYWORDS)` —— **子串包含**匹配(非前缀)。
- 异常 → `logger.debug("is_ad_url failed for %r: %s", url, exc)`,返回 False。

#### `is_platform_url(url: str) -> bool`(L113-114)
- `any(re.search(p, url) for p in PLATFORM_PATTERNS)` —— 对完整 URL 字符串做正则搜索。

### 3.3 DOM 辅助函数(纯函数,依赖 HTML 解析器,可 1:1)

> Rust 侧需选定 HTML 解析器(如 `scraper`/`lxml` 风格)。注意 BeautifulSoup `find_all` 返回**文档序**,`el.parents` 自底向上迭代,`el.get_text(" ", strip=True)` 用空格连接所有文本节点并 strip。

#### `el_text(el) -> str`(L119-120)
- `el.get_text(" ", strip=True) if el else ""`。

#### `is_content_img(img) -> bool`(L123-125)
- `src = img.get("src") or img.get("data-src") or img.get("data-lazy-src") or ""`;
- 返回 `bool(src) and not src.startswith("data:") and not src.endswith(".svg")`。

#### `_best_img_url(img, page_url: str) -> str`(L128-139)
- 若存在 `srcset` 属性:`candidates = [p.strip().split()[0] for p in srcset.split(",") if p.strip()]`,非空时返回 `urljoin(page_url, candidates[-1])` —— **取 srcset 最后一项(约定最高分辨率)**;
- 否则依次取 `src`、`data-src`、`data-lazy-src` 中第一个非空且不以 `data:` 开头的值,`urljoin(page_url, val)` 解析为绝对 URL;
- 全部失败返回 `""`。
- `urljoin` 语义为 RFC 3986 相对解析;Rust 用 `url::Url::join`。

#### `resolve_remote_reference(img, soup) -> str`(L142-170,alt 文本解析,优先级从高到低)
1. `aria-describedby` / `aria-labelledby` 属性 → 在文档中 `find(id=ref_id)`,取其 `el_text`;
2. 父级 `<figure>` 内的 `<figcaption>` 文本;
3. 父级 `<td>` 所在 `<tr>` 中**其他 `<td>`** 的 `el_text`,以 `" | "` 连接(取所有非空兄弟单元格);
4. 遍历 img 属性:任何以 `data-` 开头且属性名含 `caption`/`label`/`desc`/`title` 之一者 → `str(val).strip()`(首个非空);
5. 兜底:`img.get("title", "").strip()`。

#### `_build_tabpanel_labels(root) -> dict[str, str]`(L173-190)
- 遍历所有 `[role=tab]`:label = `el_text(tab).strip()`(空则跳过);`aria-controls` 非空 → `labels[controls] = label`;
- 遍历所有 `[role=tabpanel]`:`id` 非空且尚未在 labels 中 → 若 `aria-labelledby` 存在,`labels[panel_id] = el_text(find(id=labelledby)).strip()`。
- 返回 `{panel_id: tab_label}`。

#### `_tabpanel_info(el, root, tabpanel_labels) -> (panel_id, tab_label)`(L193-201)
- 沿 `el.parents` 向上(遇 `root` 停止),找到第一个 `role == "tabpanel"` 的祖先 → 返回 `(其 id, tabpanel_labels.get(id, ""))`;找不到返回 `("", "")`。

### 3.4 核心:构建统一 blocks[](纯函数,可 1:1)

#### `build_blocks(soup, page_url: str, source: str) -> list[dict]`(L206-258)
- **root 选择优先级**:`<main>` → `[role="main"]` → `<article>` → `<body>` → `soup`。
- 在 root 内 `find_all(["h1","h2","h3","h4","p","li","img"], recursive=True)`(**文档序**),对每个元素依次处理:
  1. `panel_id, tab_label = _tabpanel_info(...)`;
  2. 若 `tab_label.lower() in NOISE_TABPANEL_LABELS` → **跳过该元素**(整个噪音 tabpanel 内容剔除);
  3. 若 `panel_id` 首次出现 → 记入 `injected_panels`,且若 `tab_label` 非空 → **注入一个 `{"type":"heading","level":2,"text":tab_label,"source":source}` 块**(tab 标签提升为 h2);
  4. 若元素 class 与 `NOISE_CLASSES` 任一**子串匹配**(`any(cls in " ".join(el.get("class", [])) ...)`)→ 跳过;
  5. heading(`h1`-`h4`):`text = el_text(el)`;非空且 `text not in seen_text` → 记入 seen 并追加 `{"type":"heading","level":int(el.name[1]),"text":text,"source":source}`;
  6. `img`:`is_content_img` 为假 → 跳过;`_best_img_url` 为空 → 跳过;`alt = el.get("alt","").strip() or resolve_remote_reference(el, soup)`;追加 `{"type":"image","url":url,"alt":alt,"source":source,"path":None}`;
  7. 其他(`p`/`li`):`text = el_text(el)`;**仅当 `len(text) > 15`** 且未在 seen 中 → 记入 seen 并追加 `{"type":"text","text":text[:400],"source":source}`(**文本截断 400 字符**)。
- `seen_text` 集合在 heading 与 text **两类之间共享**,即同一文本无论以何种类型出现都只保留首次。
- 注意:文本长度阈值 `> 15`(严格大于);截断 `[:400]`。

#### `parse_page_html(html: str, page_url: str, source: str) -> list[dict]`(L261-267)
- `soup = BeautifulSoup(html, "html.parser")`;
- 对每个 `NOISE_IDS` 中的 id:`el = soup.find(id=noise_id)`,非空则 `el.decompose()`(整棵子树移除);
- 返回 `build_blocks(soup, page_url, source)`。

### 3.5 视频 URL 检测(纯函数,可 1:1)

#### `detect_video_urls_from_html(html: str) -> list[str]`(L272-282)
对原始 HTML 文本(不做解析)做正则搜索,按以下规则生成并**去重保序**(`list(dict.fromkeys(...))`):

| 正则 | 生成 URL |
|---|---|
| `youtube\.com/embed/([A-Za-z0-9_-]+)` | `https://www.youtube.com/watch?v=<id>` |
| `bilibili\.com/video/(BV[A-Za-z0-9]+)` | `https://www.bilibili.com/video/<bv>` |
| `aid=(\d+)` | `https://www.bilibili.com/video/av<id>` |
| `player\.vimeo\.com/video/(\d+)` | `https://vimeo.com/<id>` |

### 3.6 Playwright 抓取(网络依赖,不可 1:1 移植;Rust 侧可用 chromium/headless 方案或等价实现)

#### `async scrape_one_page(page, page_url: str, dismiss_cookie: bool = False) -> (html, video_urls, subpage_links)`(L287-340)
1. `page.goto(page_url, wait_until="networkidle", timeout=30_000)`;若响应存在且 `resp.status >= 400` → `logger.warning("[scrape] HTTP %s for %s", ...)` 并**提前返回 `("", [], [])`**;
2. `wait_for_timeout(1500)`(等待渲染);
3. 若 `dismiss_cookie=True`(仅主页面):按 `COOKIE_SELECTORS` 顺序尝试:取首个 locator,`is_visible(timeout=1000)` 为真则 `click()`,随后 `wait_for_load_state("networkidle", timeout=10_000)` 并 **break**(只点一个);每个选择器失败仅 debug 日志,继续下一个;
4. `page.evaluate("() => window.scrollTo({ top: document.body.scrollHeight, behavior: 'smooth' })")` 滚到底,`wait_for_timeout(1500)`,`html = page.content()`;
5. 提取子页链接:选择器 `"main a[href], article a[href], [role='main'] a[href], .content a[href]"` 的 href 数组;为空则回退 `"a[href]"`。对去重后的每个 link:`parsed.netloc == base_domain`(同域)且 `link != page_url` 且 `not is_utility_url(link)` 且 `not is_ad_url(link)` → 追加 `subpage_links`;
6. 返回 `(html, detect_video_urls_from_html(html), subpage_links)`。
- **任何异常 → `logger.warning("[scrape] Playwright error for %s: %s", ...)`,返回 `(html, [], subpage_links)`**(部分结果容忍)。

#### `async scrape_subpage(context, page_url: str) -> (html, video_urls)`(L343-351)
- 新建页面;若 `playwright_stealth` 可用则 `Stealth().apply_stealth_async(page)`;`scrape_one_page(page, page_url, dismiss_cookie=False)`(子页**不**关 cookie 弹窗);`finally: page.close()`。

#### `async scrape_pages_playwright(page_url: str, max_subpages: int = 5) -> (blocks, video_urls, page_title)`(L354-437)
1. 启动 Chromium:`headless=True`,args = `--disable-blink-features=AutomationControlled, --no-sandbox, --disable-setuid-sandbox, --disable-dev-shm-usage`;
2. context:`user_agent=STEALTH_UA`,viewport `1280x800`,`locale="en-US"`,extra header `Accept-Language: en-US,en;q=0.9`;
3. 主页面(stealth)→ `scrape_one_page(dismiss_cookie=True)`,`page_title = await main_page.title()`;
4. 噪音子页判定 `_is_noise_subpage(url)`:`path = urlparse(url).path.lower().rstrip("/")`,对任一 `kw ∈ NOISE_SUBPAGE_PATHS` 满足 `path == kw` 或 `path.endswith(kw)` 或 `("/" + kw.lstrip("/")) in path` → 噪音;
5. `filtered_links` 剔除噪音后取 **前 `max_subpages` 个**(`subpage_urls = filtered_links[:max_subpages]`);
6. `asyncio.gather` 并发抓所有子页,`return_exceptions=True` —— **单个子页失败不影响整体**;
7. 合并:主页面 `parse_page_html(html, page_url, "main")`;每个成功子页 `parse_page_html(sub_html, sub_url, "subpage")`(失败 → `logger.warning("[skip-error] %s: %s", ...)` 跳过;`sub_html` 为空 → 跳过;成功记 `logger.info("[found] %s — %d images", ...)`);全部 blocks 依序拼接;
8. **图片块按 URL 去重,保留首次出现**(`seen_img_urls` 集合),非 image 块不参与去重;
9. `video_urls` 全量去重保序;
10. **平台页增强**:若 `is_platform_url(page_url)` 且 `page_url not in video_urls` → `video_urls.insert(0, page_url)`(页面本身是 YouTube/Bilibili 等视频页时,把页面 URL 放到队首);
11. 返回 `(deduped_blocks, video_urls, page_title)`。

#### `scrape_page(url: str, max_subpages: int = 5) -> (blocks, video_urls, page_title)`(L440-441)
- `asyncio.run(scrape_pages_playwright(url, max_subpages=max_subpages))` —— 同步包装。

### 3.7 LLM 兜底抓取(LLM 驱动,网络依赖)

#### `scrape_page_via_llm(client: OpenAI, url: str) -> (blocks, video_urls, page_title)`(L444-466)
- 调用 `chat.completions.create`:model=`MODEL`;system=`SCRAPE_FALLBACK_PROMPT`;user=`f"Fetch and parse this page: {url}"`;`extra_body={"plugins": [{"id": "web", "max_results": 1}]}`;`temperature=0.0`;`max_tokens=6000`;
- 解析:`data = json.loads(strip_json_fence(content))`;
- `blocks = [{**b, "source": b.get("source", "main"), "path": None} for b in data.get("blocks", []) if b.get("type") in ("heading","text","image")]` —— 过滤未知类型,`source` 缺省补 `"main"`,强制 `path=None`;
- 返回 `(blocks, data.get("video_urls", []), data.get("title", ""))`;
- **任何异常 → `logger.warning("[scrape] LLM fallback also failed: %s", exc)`,返回 `([], [], "")`**。

### 3.8 CLI 入口 `main()`(L471-506)

**参数**:
| 参数 | 默认 | 说明 |
|---|---|---|
| `url`(位置参数) | 必填 | 目标页面 |
| `--slug` | `None` | 覆盖 slug;缺省 `common.url_to_slug(url)` |
| `--out` | `None` | 输出 JSON;缺省 `work/<slug>/stage_01_scrape.json` |
| `--max-subpages` | `5`(int) | 最大子页数 |
| `--no-llm-fallback` | `False`(flag) | 禁用 LLM 兜底 |

**流程**:
1. `blocks, video_urls, page_title = scrape_page(...)`;
2. **封锁检测**:`blocked_markers = ("the request is blocked", "access denied", "403 forbidden", "enable javascript")`;`img_blocks` = 全部 image 块;`text_content` = 所有 heading/text 块 text 小写后以空格连接;`is_blocked = (not img_blocks) and any(m in text_content for m in blocked_markers)`(L485-488);
3. 若 `not args.no_llm_fallback and (not blocks or is_blocked)`:`client = OpenAI(api_key=API_KEY, base_url=API_BASE)`;`fallback_blocks, fallback_videos, fallback_title = scrape_page_via_llm(...)`;若 fallback 有 blocks → **整体替换** `blocks/video_urls`;若 `fallback_title` 非空且原 `page_title` 为空 → 采纳 fallback title(L490-496);
4. `write_json(out, {"url", "slug", "title": page_title, "blocks", "video_urls"})`(L499-505);
5. `logger.info("[stage 1] wrote %s: %d blocks (%d images), title: %r", ...)`。

**错误行为**:stage 1 从不抛异常(Playwright/LLM 异常均被内部捕获),**即使抓取完全失败也写出 JSON(blocks 为空数组)**。

---

## 4. stage_02_download.py 规格(下载)

### 4.1 模块级会话

- `_fetch_session = requests.Session()`(L16-20),headers:`User-Agent: STEALTH_UA`,`Referer: https://www.google.com/`。**会话全局复用**。

### 4.2 `fetch_one(url: str) -> (url, data|None, mime|None)`(L23-40)(网络依赖)

1. `session.get(url, timeout=10, stream=True)`,`raise_for_status()`(4xx/5xx → 异常);
2. `mime = resp.headers.get("content-type", "image/jpeg").split(";")[0].strip()` —— **缺省 `image/jpeg`**,分号截断、strip;
3. 若 `mime not in SUPPORTED_MIMES` → 返回 `(url, None, None)`(**不做内容嗅探,只信响应头**);
4. 流式读取,chunk 8192;累计 `len(data) > MAX_IMAGE_BYTES`(5 MiB,**严格大于**)→ 返回 `(url, None, None)` 中止下载;
5. `Image.open(io.BytesIO(data))` 校验图片可解码,并取 `img.width`/`img.height`;**`img.width < MIN_DIMENSION and img.height < MIN_DIMENSION`(AND,双维都 <80)才拒绝**;即 800×50 的图通过、80×80 通过、79×79 拒绝;
6. 成功返回 `(url, data, mime)`。
- **任何异常(网络错误、解码失败、尺寸检查)→ 返回 `(url, None, None)`**(静默失败,无日志)。
- Rust 对应:HTTP 客户端(reqwest)+ 图片头解码(image crate 只解码 header 取宽高;注意 PIL 是惰性打开,不要求完整解码)。

### 4.3 `download_image_blocks(blocks: list[dict]) -> (new_blocks, fetched)`(L43-80)(编排确定性 + 网络)

1. `image_items = [(i, b) for i, b in enumerate(blocks) if b["type"] == "image"]`;`urls` 为其中 url(可能重复);
2. `ThreadPoolExecutor(max_workers=FETCH_WORKERS=10)` 并发 `fetch_one`;`raw[url] = (data, mime)` 仅收录 `data and mime` 均非空者(URL 为 key,重复 URL 后者覆盖,但随后按块序消费);
3. 顺序消费 `image_items`(块序):
   - `raw.get(url)` 为 `None` → `logger.debug("[skip] download failed or too small: %s", url[:80])`,跳过该 image 块;
   - `digest = sha256(data).hexdigest()`;`digest in seen_hashes` → `logger.debug("[skip] content duplicate: %s", url[:80])`,跳过(**内容去重,相同 SHA-256 只保留块序中首个**);
   - 否则:`seen_hashes.add(digest)`;`fetched[url] = (data, mime)`;`valid_block_indices.add(i)`;
4. `new_blocks = [b for i, b in enumerate(blocks) if b["type"] != "image" or i in valid_block_indices]` —— **非 image 块全保留,image 块仅保留下载成功且去重后的**;相对顺序不变;
5. 返回 `(new_blocks, fetched)`。
- **确定性**:`fetched` 按块序插入 → 后续 `save_fetched_assets` 的 `dom_NNN` 编号与块序一一对应;并发完成顺序不影响结果。

### 4.4 CLI 入口 `main()`(L83-105)

**参数**:`input_json`(位置)、`--out`(缺省 `work/<slug>/stage_02_download.json`)、`--asset-dir`(缺省 `work/<slug>/assets`)。

**流程**:`slug = data["slug"]`(**缺失 → KeyError**);`blocks, fetched = download_image_blocks(data.get("blocks", []))`;`asset_manifest = save_fetched_assets(fetched, asset_dir, "dom")`;`write_json(out, {**data, "blocks", "fetched_assets": asset_manifest, "asset_dir": asset_dir.as_posix()})`;`logger.info("[stage 2] wrote %s: %d unique image block(s) kept", ...)`。

---

## 5. stage_03_filter.py 规格(LLM 过滤)

### 5.1 `get_image_context(blocks: list[dict], idx: int) -> (heading, text_before, text_after)`(L15-35)(纯函数)

- 向后扫描 `idx-1 → 0`:取**首个** heading 文本作为 `heading`、**首个** text 文本作为 `text_before`;两者齐了即停(互不依赖先后);
- 向前扫描 `idx+1 → end`:取**首个** text 文本作为 `text_after`;
- 返回三元组;任何一项缺失为空串。

### 5.2 `filter_batch(client, batch_items, batch_images, page_title) -> list[bool]`(L38-84)(LLM 驱动)

- 构造多模态 user content:
  - 第 1 条 text:`The user is looking for screenshots that illustrate how to: "{page_title}"\n\nThere are {n} images numbered 1 to {n}. Surrounding text context is provided alongside each image.\nReply with ONLY a JSON array of {n} strings, each "KEEP" or "SKIP".`;
  - 每个图片 idx(从 1 计):
    - 上下文段:`Source: subpage (apply stricter relevance check)`(仅当 `block.source == "subpage"`);`Section heading: {heading}`;`Context before: {text_before[:200]}`;`Context after: {text_after[:200]}`(均为 **200 字符截断**);无任何段时用 `(no text context)`;
    - text 块 `Image {idx}:\n{ctx_str}` + image_url 块 `{"type":"image_url","image_url":{"url": encode_b64(data, mime)}}`;
- 调用:model=`MODEL`;system=`FILTER_PROMPT`;user=content;`temperature=0.0`;`max_tokens=128`;
- 解析:`json.loads(strip_json_fence(raw))`;返回 `[str(item).upper() == "KEEP" for item in ...]`(大小写不敏感,`"keep"`/`"KEEP"` 都算 KEEP);
- **失败兜底**:任何异常 → `logger.warning("[filter] batch failed (%s), keeping all", exc)`,返回 `[True] * len(batch_images)`(整批全保留)。

### 5.3 `filter_image_blocks(client, blocks, fetched, page_title) -> list[dict]`(L87-134)(编排确定性 + LLM)

1. 收集 `image_items`:仅 `type=="image"` **且 `url in fetched`** 的块(不在 fetched 中的 image 块跳过过滤、原样保留),带上下文三元组;
2. `image_items` 为空 → 直接返回原 `blocks`;
3. 按 `FILTER_BATCH=5` 分批(`batches`);
4. `ThreadPoolExecutor(max_workers=FILTER_WORKERS=3)` 并发提交各批;每批 `items_for_filter = [(b,h,tb,ta)...]`、`images_for_filter = [fetched[b["url"]]...]`;`future_map[future] = [block_idx...]`;
5. 结果按块索引回填 `keep_flags`;逐条 `logger.info("[%s] %s", "KEEP"/"SKIP", url[:80])`;
6. 重建:`[b for i, b in enumerate(blocks) if i not in skip_indices]`,SKIP 的 image 块被删除,其余(含全部非 image 块与未过滤的 image 块)原序保留。

### 5.4 CLI 入口 `main()`(L137-160)

**参数**:`input_json`(位置)、`--title`(缺省 `None`)、`--out`(缺省 `work/<slug>/stage_03_filter.json`)。

**流程**:
1. `title = args.title or data.get("title") or ""`;**若最终为空 → `parser.error("--title is required when the input JSON has no 'title' field.")`** —— argparse 打印 usage + 该错误消息到 stderr 并以**退出码 2** 终止(L148-149);
2. `asset_dir = Path(data.get("asset_dir") or work_path(slug, "assets"))`;`fetched = load_fetched_assets(asset_dir, data.get("fetched_assets", {}))`;
3. `client = OpenAI(api_key=API_KEY, base_url=API_BASE)`(缺少 API_KEY/BASE 的环境配置 → 在此显式失败);
4. `blocks = filter_image_blocks(client, data["blocks"], fetched, title)`;
5. `write_json(out, {**data, "title": title, "blocks": blocks})`;
6. `logger.info("[stage 3] wrote %s: %d / %d image blocks kept", out, after, before)`。

---

## 6. stage_04_save.py 规格(保存)

### 6.1 `save_image_blocks(blocks, fetched, img_dir) -> list[dict]`(L12-43)(确定性 + 文件 IO)

1. `img_dir.mkdir(parents=True, exist_ok=True)`;`counter = 0`;
2. 遍历 blocks:
   - 非 image 块 → 原样追加;
   - image 块:`url = block["url"]`;**`url not in fetched` → `logger.warning("[warn] no fetched data for %s, skipping", url[:80])` 并丢弃该块**(不计数);
   - 否则:`ext = Path(urlparse(url).path).suffix.lower()`;若 `ext not in SUPPORTED_EXTS` → `ext = MIME_TO_EXT.get(mime, ".png")`(与 `image_ext` 同逻辑但内联实现);
   - `dest = img_dir / f"img_{counter:02d}{ext}"` —— **2 位零填充序号**;
   - `dest.write_bytes(data)`;`logger.info("[save] img_%02d%s ← %s", counter, ext, url[:60])`;
   - 追加 `{**block, "path": dest}`(Path 对象);`counter += 1`;
3. 返回新 blocks。

**文件名生成规则**:`img_<counter:02d><ext>`,`counter` 从 0 起、仅对成功保存的 image 块递增。

### 6.2 CLI 入口 `main()`(L46-72)

**参数**:`input_json`(位置)、`--slug`(缺省 `None`,否则 `data["slug"]`,**缺失 → KeyError**)、`--skills-dir`(缺省 `"skills"`)、`--out`(缺省 `work/<slug>/stage_04_save.json`)。

**流程**:
- `skill_dir = Path(args.skills_dir) / slug`;`img_dir = skill_dir / "references"`;`asset_dir = Path(data.get("asset_dir") or work_path(slug, "assets"))`;`fetched = load_fetched_assets(...)`;
- `blocks = save_image_blocks(data.get("blocks", []), fetched, img_dir)`;
- `write_json(out, {**data, "slug", "skills_dir": args.skills_dir(原样字符串), "skill_dir": skill_dir.as_posix(), "blocks": blocks_with_paths_as_str(blocks)})`;
- `logger.info("[stage 4] wrote %s: %d image(s) saved to %s", ...)`。

---

## 7. stage_05_generate.py 规格(生成 SKILL.md)

### 7.1 `blocks_for_llm(blocks, skill_dir) -> list[dict]`(L16-37)(确定性)

- 对每个 block:
  - image 块:`path = b.get("path")`;为 `None` → **丢弃该块**(不给 LLM);否则 `path = Path(str(path))`;`rel = path.relative_to(skill_dir).as_posix()`(`ValueError`(不在 skill_dir 下)→ 回退 `rel = path.name`);输出 `{"type":"image","path":rel,"alt":b.get("alt",""),"source":b.get("source","main")}` —— **丢弃 url、相对 skill_dir 化路径**;
  - 其他块:`{k: v for k, v in b.items() if k not in ("path", "url")}` —— **剥离 `path` 与 `url` 字段**(heading/text 块只留 type/text/source/level)。
- 输出即 LLM 输入格式;image 块的 `path` 形如 `references/img_00.png`。

### 7.2 `call_skill_agent(client, title, blocks_llm) -> str`(L40-52)(LLM 驱动)

- `blocks_json = json.dumps(blocks_llm, ensure_ascii=False, indent=2)`;
- `user_msg = f"Title: {title}\n\n=== BLOCKS ===\n{blocks_json}"`;
- 调用:model=`MODEL`;system=`SKILL_PROMPT`;user=user_msg;`temperature=0.2`;`max_tokens=8192`;
- 返回 `resp.choices[0].message.content.strip()`。
- **错误行为:无 try/except —— LLM 调用异常直接向上传播,进程崩溃**(与 stage 1/3 的宽容策略不同)。Rust 侧必须保留"LLM 失败即失败"语义,给出显式错误。

### 7.3 `append_reference_files(skill_md: str, blocks_llm) -> str`(L55-65)(确定性)

- `images = [b for b in blocks_llm if b["type"] == "image" and b.get("path")]`;为空 → 原样返回;
- 否则在文末追加(`skill_md.rstrip() + "\n\n" + ... + "\n"`):

```
## Reference Files

For visual reference, the following screenshots are available:
- `references/img_00.png` — <alt 或 "screenshot">
...
```

- 每行 `- \`{path}\` — {description}`,`description = alt or "screenshot"`。

### 7.4 CLI 入口 `main()`(L68-98)

**参数**:`input_json`(位置)、`--out`(缺省 `skill_dir/SKILL.md`)、`--clean`(flag,"Delete work/<slug>/ after successful generation.")。

**流程**:
1. `skill_dir = Path(data["skill_dir"])`(**缺失 → KeyError**);`out_path = args.out 或 skill_dir / "SKILL.md"`;
2. `blocks = blocks_with_paths_as_path(data.get("blocks", []))`;
3. `blocks_llm = blocks_for_llm(blocks, skill_dir)`;
4. `valid_paths = {b["path"] for b in blocks_llm if b["type"] == "image" and b.get("path")}`;
5. `client = OpenAI(api_key=API_KEY, base_url=API_BASE)`;`skill_md = call_skill_agent(client, data.get("title", ""), blocks_llm)`;
6. **确定性后处理(顺序固定)**:
   - `skill_md = strip_hallucinated_images(skill_md, valid_paths)`(删除 LLM 幻觉图片引用);
   - `skill_md = append_reference_files(skill_md, blocks_llm)`(追加 Reference Files);
7. `out_path.parent.mkdir(parents=True, exist_ok=True)`;`write_text(skill_md, encoding="utf-8")`;`logger.info("[stage 5] wrote %s", out_path)`;
8. 若 `--clean`:`work_dir = work_path(data["slug"], "")`;存在则 `shutil.rmtree(work_dir)`(`logger.info("[stage 5] cleaned %s", ...)`)。**仅成功写 SKILL.md 后才清理**。

---

## 8. 阶段间输入/输出契约汇总表

| 阶段 | 输入 | 输出文件 | 附带落盘 | 说明 |
|---|---|---|---|---|
| 1 抓取 | URL(CLI) | `work/<slug>/stage_01_scrape.json` | 无 | 产出 `url/slug/title/blocks/video_urls` |
| 2 下载 | stage_01 JSON | `work/<slug>/stage_02_download.json` | `work/<slug>/assets/dom_NNN.ext` | 追加 `fetched_assets`、`asset_dir`;失败的 image 块被删 |
| 3 过滤 | stage_02 JSON + assets | `work/<slug>/stage_03_filter.json` | 无(读 assets) | 重写 `title`;SKIP 的 image 块被删 |
| 4 保存 | stage_03 JSON + assets | `work/<slug>/stage_04_save.json` | `skills/<slug>/references/img_NN.ext` | 填充 image 块 `path`;追加 `skills_dir`、`skill_dir`;不在 fetched 的 image 块被删 |
| 5 生成 | stage_04 JSON | `skills/<slug>/SKILL.md` | 无 | 可选 `--clean` 删除 `work/<slug>/` |

跨阶段贯通字段:`url`、`slug`、`title`(stage3 重写)、`video_urls`、`fetched_assets`(stage2+)、`asset_dir`(stage2+)。

---

## 9. 确定性规则汇总(Rust 必须 1:1 实现)

### 9.1 URL 规范化与判定

- slug 生成:`url_to_slug`(netloc+path → 去首尾 `/` → `slugify`);`slugify` = lower → 删 `[^\w\s-]` → `[\s_-]+` 折叠为 `_` → 截 80 字符。
- 图片相对 URL → 绝对 URL:`urljoin`(RFC 3986);srcset 取**最后**一项。
- 子页链接判定:同域(`netloc == base_domain`)+ 非自身(`link != page_url`)+ 非 utility(`is_utility_url`)+ 非广告(`is_ad_url`)。
- 广告判定:域名后缀匹配 AD_DOMAINS(剥 `www.`、排除单标签)或路径含 AD_PATH_KEYWORDS 子串。
- 噪音子页:path 等于/以关键字结尾/路径中含 `/<kw>`(`_is_noise_subpage`,L385-390)。
- 平台页:`PLATFORM_PATTERNS` 任一正则命中;命中且自身不在 video_urls 时插到队首。

### 9.2 内容长度 / 数量限制

| 规则 | 值 | 位置 |
|---|---|---|
| text 块最小长度 | `len(text) > 15` 才成块 | stage_01 L254 |
| text 块截断 | `text[:400]` | stage_01 L256 |
| 过滤上下文截断 | `text_before[:200]` / `text_after[:200]` | stage_03 L63-64 |
| 图片最小尺寸 | 双维均 `< 80` 才拒(AND) | stage_02 L36-37 |
| 图片最大字节 | `> 5 MiB` 拒(严格大于) | stage_02 L33-34 |
| 最大子页数 | `max_subpages`(默认 5),噪音剔除后取前 N | stage_01 L393 |
| 下载并发 | `FETCH_WORKERS=10` | stage_02 L49 |
| 过滤批量 / 并发 | `FILTER_BATCH=5` / `FILTER_WORKERS=3` | stage_03 L107/L111 |
| 视频帧常量 | `VIDEO_FRAMES=4`(未使用,死常量) | common L32 |

### 9.3 文件名生成

| 场景 | 格式 | 位置 |
|---|---|---|
| stage 2 资产(assets 目录) | `{prefix}_{idx:03d}{ext}`,prefix=`"dom"`,idx 按 fetched 插入序 | common L252(stage_02 L96 传入 `"dom"`) |
| stage 4 参考图(references 目录) | `img_{counter:02d}{ext}`,counter 按成功保存序 | stage_04 L37 |
| 扩展名来源 | URL path 后缀(小写)∈ SUPPORTED_EXTS,否则 MIME→ext,再否则 `.png` | common L235-239 / stage_04 L34-36 |

### 9.4 SKILL.md 骨架(由 SKILL_PROMPT 约束 + 代码后处理)

```
---
name: <snake_case_skill_name>
description: <1-3 句英文>
---

# <Skill Name>

## Steps

### <h2 文本>                ← 存在 h2 时,每个 h2 一组,组内步骤从 1 重新编号
#### <h3 文本>              ← 该 h2 组内存在 h3 时,每个 h3 一小节,小节内步骤从 1 重新编号
1. <动词> **<UI 标签>**
![alt text](references/img_NN.ext)
2. ...
### <下一个 h2>
...

## Reference Files            ← 仅当存在图片块时由代码追加(stage_05 L55-65)

For visual reference, the following screenshots are available:
- `references/img_00.png` — <alt 或 "screenshot">
```

- **分组规则(提示词内,自上而下取首个命中)**:① 有 h2:每个 h2 → `###`,组内 h3 → `####`,编号各自从 1 重启;② 无 h2 有 h3:每个 h3 → `###`,编号每组分段重启;③ 无 h2 无 h3:扁平编号列表,无 `###`/`####`。
- 步骤行格式:`1. <verb> **<UI label>**`;图片独占一行且前后各空一行;图片路径必须逐字复制自 blocks 的 `path`。
- 这些规则由 LLM 按 SKILL_PROMPT 执行;代码侧确定性后处理仅两步:去幻觉图片引用(`strip_hallucinated_images`)+ 追加 `## Reference Files`。

### 9.5 去重与过滤判据

| 判据 | 规则 | 位置 |
|---|---|---|
| heading/text 文本去重 | `seen_text` 集合,跨类型共享,保留首次 | stage_01 L239-256 |
| 图片块 URL 去重 | 跨主/子页合并后按 URL 保留首次 | stage_01 L424-431 |
| 视频 URL 去重 | `dict.fromkeys` 保序 | stage_01 L282/L433 |
| 图片内容去重 | SHA-256 摘要,块序首个胜出 | stage_02 L67-71 |
| 噪音元素 | `NOISE_IDS` decompose;`NOISE_CLASSES` 子串匹配跳过;`NOISE_TABPANEL_LABELS` 跳过整个 tabpanel;噪音 tabpanel 的标签本身会被注入为 h2(仅内容 tabpanel) | stage_01 L261-267/L224-235 |
| 封锁检测 | 无图片块 且 文本含任一 marker(`the request is blocked`/`access denied`/`403 forbidden`/`enable javascript`) | stage_01 L485-488 |

---

## 10. 确定性 vs LLM 驱动 vs 网络依赖 划分

### 10.1 纯函数(确定性,可 1:1 移植,无 IO 或仅本地文件 IO)

| 函数 | 文件:行号 | 备注 |
|---|---|---|
| `load_json` / `write_json` | common.py:196-202 | 本地文件 IO;`ensure_ascii=False` + `indent=2` |
| `strip_json_fence` | common.py:205-209 | 正则去围栏 |
| `encode_b64` | common.py:212-213 | 标准 base64 |
| `slugify` / `url_to_slug` / `work_path` / `image_ext` | common.py:218-239 | 路径/slug |
| `save_fetched_assets` / `load_fetched_assets` | common.py:244-264 | 本地文件 IO,顺序枚举 |
| `blocks_with_paths_as_str` / `_as_path` | common.py:269-286 | Path↔str |
| `strip_hallucinated_images` | common.py:289-303 | 正则 + 空行折叠 |
| `is_utility_url` / `is_ad_url` / `is_platform_url` | stage_01:91-114 | URL 判定 |
| `el_text` / `is_content_img` / `_best_img_url` / `resolve_remote_reference` | stage_01:119-170 | DOM 提取(需 HTML 解析器) |
| `_build_tabpanel_labels` / `_tabpanel_info` | stage_01:173-201 | DOM 结构 |
| `build_blocks` / `parse_page_html` | stage_01:206-267 | 核心 blocks 构建 |
| `detect_video_urls_from_html` | stage_01:272-282 | 正则 |
| `get_image_context` | stage_03:15-35 | 上下文扫描 |
| `blocks_for_llm` / `append_reference_files` | stage_05:16-37 / 55-65 | 输入准备/输出后处理 |
| `filter_image_blocks` 的编排骨架 | stage_03:87-134 | 分批/并发/重建(决策本身是 LLM) |

### 10.2 网络依赖(HTTP/浏览器抓取,不可 1:1;Rust 侧用等价实现)

| 函数 | 文件:行号 | 依赖 |
|---|---|---|
| `scrape_one_page` / `scrape_subpage` / `scrape_pages_playwright` / `scrape_page` | stage_01:287-441 | Playwright/Chromium(JS 渲染、cookie 处理、滚动、networkidle、链接提取) |
| `_fetch_session` / `fetch_one` | stage_02:16-40 | HTTP GET、流式下载、内容类型头 |

> 移植策略:stage_01 的浏览器部分可由 Rust 侧选择 headless chromium 方案(chromiumoxide/playwright-rust)或"HTTP+静态解析+document 未渲染内容"的降级实现;但**行为契约(状态码≥400 提前返回、cookie 选择器、滚动等待、子页数量上限、平台页插队)必须保持一致**。

### 10.3 LLM 驱动(依赖外部模型;Rust 侧不可用时**必须显式报错**)

| 函数 | 文件:行号 | 说明 |
|---|---|---|
| `scrape_page_via_llm` | stage_01:444-466 | stage_01 的兜底(仅当 blocks 为空或检测到封锁,且未传 `--no-llm-fallback`) |
| `filter_batch`(含 `filter_image_blocks` 的决策) | stage_03:38-84 | 视觉 KEEP/SKIP;单批失败 → 该批全保留(keep-all 兜底) |
| `call_skill_agent` | stage_05:40-52 | SKILL.md 生成;**无兜底,失败即崩溃** |

**Rust 侧硬性要求**:
1. LLM 客户端所需配置(`API_BASE`/`API_KEY`/`MODEL_NAME`)在**进程启动时**校验,缺失 → 显式错误(消息指明缺失项),而不是运行到中途才失败;
2. stage_01:若 LLM 兜底被触发(需要兜底)但 LLM 不可用 → **显式报错并终止**(Python 行为是静默返回空,但按本规格的移植要求,LLM 不可用必须显式报错;若保留 Python 的宽容语义,须以配置开关区分);
3. stage_03:单批 LLM 调用失败可保留 keep-all 兜底(与 Python 一致),但**若 LLM 客户端整体不可用(未配置/网络级失败持续),应显式报错**,不允许静默全保留产出劣质结果;
4. stage_05:LLM 失败必须作为硬错误返回(与 Python 无 try/except 语义一致),错误消息应包含 HTTP 状态/API 错误体。

### 10.4 依赖关系图

```
[stage_01] 确定性(DOM/URL/正则) ── 网络(Playwright) ── LLM(兜底,可禁用)
[stage_02] 确定性(去重/限流逻辑) ── 网络(HTTP 下载)      [无 LLM]
[stage_03] 确定性(分批/上下文/重建) ── LLM(视觉过滤)      [无网络直连]
[stage_04] 确定性(命名/复制/落盘)                         [无网络、无 LLM]
[stage_05] 确定性(路径化/后处理) ── LLM(生成 SKILL.md)    [无网络直连]
```

---

## 11. Rust 移植注意点(差异与陷阱)

1. **`\w` 的 Unicode 语义**:`slugify` 的 `[^\w\s-]` 依赖 Unicode 单词字符(含 CJK)。Rust `regex` crate 默认 `\w`/`\s` 即 Unicode 感知,可直接等价;若使用 ASCII-only 配置则行为不同,禁止。
2. **`urlparse` vs `url` crate**:Python 容错、不规范化 host 大小写;Rust `url::Url` 严格校验并可能规范化。`url_to_slug`/`is_utility_url` 等要求宽松行为,Rust 侧建议实现或选用宽松解析,并对解析失败返回 Python 等价的降级值(如 `is_utility_url` 返回 False)。
3. **`urljoin`**:用 `url::Url::join` 等价(RFC 3986)。
4. **图片尺寸校验**:PIL 惰性打开仅需头信息;Rust `image` crate 解码 header 取宽高即可,不必全解码;注意 `Image.open` 失败(非图片数据)在 Python 中由 `except Exception` 吞掉返回 `(url, None, None)`,Rust 侧同样把解码失败当作"下载失败"。
5. **MIME 判定**:只信任 `Content-Type` 响应头(缺省 `image/jpeg`,分号截断、strip、小写),**不做内容嗅探**。
6. **并发确定性**:stage_02 的 `raw` 填充顺序无关紧要(按 URL 查);`fetched` 必须按 **block 顺序**插入以保证 `dom_NNN` 编号与块序一致;stage_03 的 `keep_flags` 按块索引回填,重建结果与并发完成顺序无关。
7. **阶段输出幂等性**:同一输入 JSON 重跑 stage 2-5,产物应逐字节一致(LLM 部分除外,因模型输出非确定)。
8. **路径字段类型流转**:stage_04 写 JSON 时 path 是**相对 CWD 的字符串**(`skills/<slug>/references/img_NN.png`);stage_05 读回后 `Path(str(path))` 再 `relative_to(skill_dir)` 得到 `references/img_NN.png` 传给 LLM。Rust 侧必须复刻这个"先绝对化/相对化再还原"的转换链,且 `relative_to` 失败(ValueError)时回退 `path.file_name()`。
9. **JSON 输出格式**:`ensure_ascii=False` + `indent=2`;`serde_json::to_string_pretty` 满足缩进,注意关闭 ASCII 转义。
10. **`--clean` 时机**:仅当 SKILL.md 成功写出后执行;`work_path(slug, "")` 即 `work/<slug>` 目录。
11. **错误消息 1:1 保留**:`parser.error("--title is required when the input JSON has no 'title' field.")`(退出码 2)、`KeyError: 'slug'`/`'skill_dir'`、环境变量缺失的 `KeyError`、各 `logger.warning` 文案(见 §12)应尽量复刻,便于行为比对测试。
12. **`save_fetched_assets`/`load_fetched_assets` 的 manifest key 是原始 URL**:JSON 对象 key 为 URL 字符串;URL 含特殊字符时序列化仍为普通字符串 key,反序列化后按原样匹配 blocks 中的 `url` 字段 —— 二者必须逐字节一致(注意 stage_01 的 URL 已过 `urljoin` 规范化)。

---

## 12. 日志/错误消息清单(供 1:1 对齐)

| 级别 | 消息 | 位置 |
|---|---|---|
| KeyError | `'API_BASE'` / `'API_KEY'` / `'MODEL_NAME'`(导入时) | common.py:14-16 |
| — | `[scrape] HTTP %s for %s` | stage_01:296 |
| — | `[scrape] Dismissed cookie banner (%s)`(debug) | stage_01:306 |
| — | `[scrape] Cookie selector %s failed: %s`(debug) | stage_01:310 |
| — | `[scrape] Link extraction failed: %s`(debug) | stage_01:334 |
| — | `[scrape] Playwright error for %s: %s` | stage_01:339 |
| — | `[subpages] Found %d candidate subpages (%d noise filtered)` | stage_01:394-395 |
| — | `[skip-error] %s: %s` | stage_01:412 |
| — | `[found] %s — %d images` | stage_01:419 |
| — | `[scrape] Direct fetch blocked — falling back to LLM web plugin...` | stage_01:445 |
| — | `[scrape] LLM fallback also failed: %s` | stage_01:465 |
| — | `[stage 1] wrote %s: %d blocks (%d images), title: %r` | stage_01:506 |
| — | `[skip] download failed or too small: %s`(debug) | stage_02:64 |
| — | `[skip] content duplicate: %s`(debug) | stage_02:69 |
| — | `[stage 2] wrote %s: %d unique image block(s) kept` | stage_02:105 |
| — | `[filter] batch failed (%s), keeping all` | stage_03:83 |
| — | `[%s] %s`(KEEP/SKIP + url[:80]) | stage_03:130 |
| — | `stage_03_filter.py: error: --title is required when the input JSON has no 'title' field.`(退出码 2) | stage_03:149 |
| — | `[stage 3] wrote %s: %d / %d image blocks kept` | stage_03:160 |
| — | `[warn] no fetched data for %s, skipping` | stage_04:30 |
| — | `[save] img_%02d%s ← %s` | stage_04:39 |
| — | `[stage 4] wrote %s: %d image(s) saved to %s` | stage_04:72 |
| — | `[stage 5] wrote %s` | stage_05:92 |
| — | `[stage 5] cleaned %s` | stage_05:98 |

---

## 13. 行号索引(功能点 → 源文件位置)

### common.py(303 行)
| 功能点 | 行号 |
|---|---|
| ROOT / .env 加载 / 环境变量 | 11-16 |
| STEALTH_UA | 18-22 |
| 支持扩展名/MIME/尺寸/字节/并发/批量常量 | 24-32 |
| FILTER_PROMPT | 36-62 |
| SKILL_PROMPT | 64-167 |
| SCRAPE_FALLBACK_PROMPT | 169-191 |
| load_json / write_json | 196-202 |
| strip_json_fence | 205-209 |
| encode_b64 | 212-213 |
| slugify | 218-222 |
| url_to_slug | 225-228 |
| work_path | 231-232 |
| image_ext | 235-239 |
| save_fetched_assets | 244-256 |
| load_fetched_assets | 259-264 |
| blocks_with_paths_as_str / _as_path | 269-286 |
| strip_hallucinated_images | 289-303 |

### stage_01_scrape.py(510 行)
| 功能点 | 行号 |
|---|---|
| 黑名单/模式常量(UTILITY/AD/PLATFORM/COOKIE/NOISE) | 24-86 |
| is_utility_url / is_ad_url / is_platform_url | 91-114 |
| el_text / is_content_img / _best_img_url | 119-139 |
| resolve_remote_reference | 142-170 |
| _build_tabpanel_labels / _tabpanel_info | 173-201 |
| build_blocks | 206-258 |
| parse_page_html | 261-267 |
| detect_video_urls_from_html | 272-282 |
| scrape_one_page(Playwright) | 287-340 |
| scrape_subpage | 343-351 |
| scrape_pages_playwright | 354-437 |
| scrape_page(同步包装) | 440-441 |
| scrape_page_via_llm(LLM 兜底) | 444-466 |
| main(CLI:slug/out/封锁检测/LLM 回退/写 JSON) | 471-506 |

### stage_02_download.py(109 行)
| 功能点 | 行号 |
|---|---|
| 会话与请求头 | 16-20 |
| fetch_one(下载/尺寸/字节/解码校验) | 23-40 |
| download_image_blocks(并发下载 + SHA-256 去重 + 块重建) | 43-80 |
| main(CLI:out/asset-dir/写 JSON) | 83-105 |

### stage_03_filter.py(164 行)
| 功能点 | 行号 |
|---|---|
| get_image_context(前后文扫描) | 15-35 |
| filter_batch(LLM 视觉批量判定 + keep-all 兜底) | 38-84 |
| filter_image_blocks(分批/并发/重建) | 87-134 |
| main(CLI:title 必填校验/读资产/写 JSON) | 137-160 |

### stage_04_save.py(76 行)
| 功能点 | 行号 |
|---|---|
| save_image_blocks(命名 img_NN + 落盘 + path 回填) | 12-43 |
| main(CLI:skills-dir/img_dir/写 JSON) | 46-72 |

### stage_05_generate.py(102 行)
| 功能点 | 行号 |
|---|---|
| blocks_for_llm(路径相对化 + 剥离 url) | 16-37 |
| call_skill_agent(LLM 生成,无兜底) | 40-52 |
| append_reference_files(追加 Reference Files) | 55-65 |
| main(CLI:后处理/写 SKILL.md/--clean) | 68-98 |

---

## 14. 移植验收要点(建议)

1. 对相同输入 JSON,stage 2/4 的产物(SKILL.md 之外的 JSON、图片文件、文件名)与 Python 逐字节一致;
2. stage_01 的 DOM 构建逻辑可用 Python 产出的 fixture(HTML → blocks JSON)做黄金样本测试;
3. LLM 调用参数(模型、temperature、max_tokens、system prompt 文本)逐字段一致;
4. 错误路径:缺失环境变量、缺失 `title`、缺失 `slug`/`skill_dir`、LLM 不可用时的报错消息与退出码对齐;
5. 并发参数(10/5/3)与超时(HTTP 10s、goto 30s、cookie 1s/10s)原样保留。
