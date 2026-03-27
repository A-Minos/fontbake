# fontbake

一个用 Rust 编写的字体构建工具，支持 CLI 和浏览器（WASM）双端使用。

核心能力：
- 解析 Java Hiero `.hiero` 配置文件，生成 BMFont 格式的位图字体（`.fnt` + `.png`）
- 支持多个 fallback 字体源（主字体缺字时自动回退）
- 支持导入现有 BMFont（`.fnt` + `.png`），拆分 glyph 后重新组合
- 支持将 outline 字体与已有 bitmap 字体混合打包
- 内置 DistanceField（SDF）效果，移植自 libGDX `DistanceFieldGenerator`

## 安装

### 从源码编译

需要 Rust 1.85+（edition 2024）。

```bash
git clone <repo-url> fontbake
cd fontbake
cargo build --release
```

编译产物在 `target/release/fontbake`。

### WASM 构建

```bash
# 需要先安装 wasm-pack
cargo install wasm-pack

# 构建 WASM 包
wasm-pack build crates/fontbake-wasm --target web
```

构建产物在 `crates/fontbake-wasm/pkg/`，可直接在浏览器中使用。

## 项目结构

```
fontbake/
├── Cargo.toml                    # workspace 根配置
├── crates/
│   ├── fontbake-core/            # 核心库（纯 Rust，无文件系统依赖）
│   │   └── src/
│   │       ├── config/           # .hiero 配置解析
│   │       ├── model/            # 内部数据模型
│   │       ├── source/           # 字体源（outline + BMFont 导入）
│   │       ├── raster/           # 光栅化（outline → alpha mask）
│   │       ├── effect/           # 效果处理（DistanceField SDF）
│   │       ├── pack/             # Atlas 打包（Hiero 风格行式排列）
│   │       ├── export/           # BMFont 导出（text .fnt + PNG）
│   │       └── pipeline/         # 构建流水线入口
│   ├── fontbake-cli/             # CLI 命令行工具
│   └── fontbake-wasm/            # WASM 浏览器绑定
├── fixtures/                     # 测试用固定资产
└── scripts/oracle/               # Java 参考输出生成脚本
```

## CLI 使用

fontbake 提供四个子命令：`build`、`import`、`merge`、`inspect`。

### build — 从 .hiero 配置构建字体

从一个 `.hiero` 配置文件出发，加载指定的 TTF/OTF 字体，执行光栅化 + SDF 效果 + 打包，输出 BMFont 格式。

```bash
fontbake build <配置文件路径> [选项]
```

参数：
- `<CONFIG>` — `.hiero` 配置文件路径（必填）
- `-o, --output <目录>` — 输出目录，默认当前目录

示例：

```bash
# 使用 HUN2 配置构建字体，输出到 output/ 目录
fontbake build HUN2.with.fallback.hiero -o output/

# 输出文件：
#   output/HUN2.fnt      — BMFont 文本格式描述文件
#   output/HUN2.png      — Atlas 纹理页（可能有多页：HUN2.png, HUN2_1.png, ...）
```

`.hiero` 配置文件格式说明：

```ini
# 字体基本信息
font.name=HUN2
font.size=52
font.bold=false
font.italic=false
font.gamma=1.8
font.mono=false

# 主字体文件（TTF/OTF）
font2.file=/path/to/your/font.ttf
font2.use=true

# 内边距
pad.top=4
pad.right=4
pad.bottom=4
pad.left=4
pad.advance.x=-8
pad.advance.y=-8

# Atlas 页面尺寸
glyph.page.width=1024
glyph.page.height=1024

# 要包含的字符集（直接写字符）
glyph.text=ABCDEFG0123456789你好世界

# 渲染模式（目前只支持 0 = Java 路径）
render_type=0

# Fallback 字体（主字体缺字时按顺序回退）
fallback.font.0=/path/to/fallback1.otf
fallback.font.1=/path/to/fallback2.ttf

# DistanceField 效果配置
effect.class=com.badlogic.gdx.tools.hiero.unicodefont.effects.DistanceFieldEffect
effect.Color=ffffff
effect.Scale=32
effect.Spread=3.5
```

关键配置项说明：

| 配置项 | 说明 |
|--------|------|
| `font2.file` | 主字体文件路径（TTF/OTF） |
| `fallback.font.N` | 第 N 个 fallback 字体路径，N 从 0 开始，按顺序查找 |
| `font.size` | 字体大小（像素） |
| `glyph.text` | 需要生成的字符集，直接写字符即可 |
| `glyph.page.width/height` | Atlas 纹理页尺寸 |
| `pad.*` | 每个 glyph 四周的内边距 |
| `pad.advance.x/y` | xadvance/yadvance 的额外调整值 |
| `effect.Scale` | SDF 中间 mask 的放大倍数（越大越精细，越慢） |
| `effect.Spread` | SDF 扩散半径（像素），影响边缘柔和程度 |
| `effect.Color` | 输出颜色（6 位十六进制，如 `ffffff`） |
| `render_type` | 渲染模式，目前只支持 `0`（Java 路径） |

Fallback 机制：
- 对 `glyph.text` 中的每个字符，按 `主字体 → fallback.font.0 → fallback.font.1 → ...` 的顺序查找
- 第一个包含该字符的字体胜出（first-hit-wins）
- 每个 glyph 会记录来源字体，可通过 `inspect` 查看

### import — 导入现有 BMFont

将一个已有的 BMFont（`.fnt` + `.png`）导入系统，解析所有 glyph 信息并重新导出。

```bash
fontbake import <FNT文件路径> [选项]
```

参数：
- `<FNT>` — `.fnt` 文件路径（必填），对应的 `.png` 文件需在同目录下
- `-o, --output <目录>` — 输出目录，默认当前目录

示例：

```bash
# 导入 hun.fnt（会自动读取同目录下的 hun.png）
fontbake import hun.fnt -o reimported/

# 输出文件：
#   reimported/hun_reimport.fnt
```

导入特性：
- 只支持文本格式的 `.fnt`（AngelCode BMFont text format）
- 导入的 bitmap glyph 像素直通，不会被重新光栅化或 SDF 处理
- 保留原始 metrics（xoffset, yoffset, xadvance）和 kerning 信息
- 支持多页 atlas（多个 `.png` 文件）

### merge — 合并多个 BMFont

将多个 BMFont 源合并为一个统一的字体输出。适用于：将不同来源的 glyph 组合到一个 atlas 中。

```bash
fontbake merge <FNT文件...> [选项]
```

参数：
- `<SOURCES>...` — 一个或多个 `.fnt` 文件路径（必填）
- `-n, --name <名称>` — 输出字体名称，默认 `merged`
- `--page-width <宽度>` — Atlas 页宽度，默认 `1024`
- `--page-height <高度>` — Atlas 页高度，默认 `1024`
- `-o, --output <目录>` — 输出目录，默认当前目录

示例：

```bash
# 合并两个 BMFont 源
fontbake merge latin.fnt cjk.fnt -n combined -o output/

# 合并三个源，指定 atlas 尺寸
fontbake merge base.fnt extra.fnt symbols.fnt \
  -n myfont \
  --page-width 2048 \
  --page-height 2048 \
  -o output/

# 输出文件：
#   output/combined.fnt
#   output/combined.png（可能有多页）
```

合并规则：
- 按输入顺序确定优先级，第一个源优先
- 同一 codepoint 在多个源中存在时，第一个源的 glyph 胜出
- 所有 glyph 会被重新打包到新的 atlas 页中
- 导入的 bitmap glyph 不会被重新处理，像素直通

### inspect — 检查配置或字体文件

以 JSON 格式输出 `.hiero` 配置或 `.fnt` 文件的解析结果，用于调试和验证。

```bash
fontbake inspect <文件路径>
```

参数：
- `<FILE>` — `.hiero` 或 `.fnt` 文件路径（必填）

示例：

```bash
# 检查 .hiero 配置
fontbake inspect HUN2.with.fallback.hiero

# 输出示例（JSON）：
# {
#   "font_name": "HUN2",
#   "font_size": 52,
#   "primary_font_path": "/path/to/hun2.ttf",
#   "fallback_font_paths": ["/path/to/chinese_font.otf"],
#   "effects": [{"DistanceField": {"color": "ffffff", "scale": 32, "spread": 3.5}}],
#   ...
# }

# 检查 .fnt 文件
fontbake inspect hun.fnt

# 输出示例（JSON）：
# {
#   "info": {"face": "HUN2", "size": 52, ...},
#   "common": {"lineHeight": 52, "base": 40, "scaleW": 512, ...},
#   "chars": [{"id": 65, "x": 95, "y": 49, "width": 36, ...}, ...],
#   ...
# }
```

可以配合 `jq` 做进一步查询：

```bash
# 查看所有 glyph 的 codepoint
fontbake inspect hun.fnt | jq '.chars[].id'

# 查看 fallback 字体列表
fontbake inspect config.hiero | jq '.fallback_font_paths'

# 统计 glyph 数量
fontbake inspect hun.fnt | jq '.chars | length'
```

## WASM / 浏览器使用

fontbake-wasm 提供以下 JavaScript API，所有输入输出都是 bytes/strings/JSON，不涉及文件系统。

### 初始化

```javascript
import init, { parse_hiero, parse_bmfont, build_font, import_bmfont } from './fontbake_wasm.js';

await init();
```

### parse_hiero(config_text) → string

解析 `.hiero` 配置文本，返回 `BuildSpec` 的 JSON 字符串。

```javascript
const configText = await fetch('HUN2.with.fallback.hiero').then(r => r.text());
const specJson = parse_hiero(configText);
const spec = JSON.parse(specJson);
console.log(spec.font_name);           // "HUN2"
console.log(spec.fallback_font_paths); // ["/path/to/chinese_font.otf"]
```

### parse_bmfont(fnt_text) → string

解析文本格式 `.fnt` 文件，返回 `BmFont` 结构的 JSON 字符串。

```javascript
const fntText = await fetch('hun.fnt').then(r => r.text());
const bmfontJson = parse_bmfont(fntText);
const bmfont = JSON.parse(bmfontJson);
console.log(bmfont.chars.length); // 180
```

### build_font(config_text, primary_font, fallback_fonts_json) → object

从配置构建完整字体。

参数：
- `config_text` — `.hiero` 配置文本
- `primary_font` — 主字体的 `Uint8Array`（TTF/OTF 文件内容）
- `fallback_fonts_json` — fallback 字体的 JSON 字符串，格式：`[{"name": "fallback.otf", "data": [byte, byte, ...]}]`

返回值：
```javascript
{
  fnt_text: "info face=...",     // 生成的 .fnt 文本内容
  page_pngs: [Uint8Array, ...],  // Atlas PNG 页面（二进制）
  glyph_count: 180               // 生成的 glyph 数量
}
```

完整示例：

```javascript
// 读取配置和字体文件
const configText = await fetch('config.hiero').then(r => r.text());
const primaryFont = new Uint8Array(await fetch('main.ttf').then(r => r.arrayBuffer()));
const fallbackFont = new Uint8Array(await fetch('fallback.otf').then(r => r.arrayBuffer()));

// 构建 fallback JSON
const fallbackJson = JSON.stringify([
  { name: "fallback.otf", data: Array.from(fallbackFont) }
]);

// 构建字体
const result = build_font(configText, primaryFont, fallbackJson);

// 使用结果
console.log(`生成了 ${result.glyph_count} 个 glyph`);

// 下载 .fnt 文件
const fntBlob = new Blob([result.fnt_text], { type: 'text/plain' });
downloadBlob(fntBlob, 'output.fnt');

// 下载 PNG atlas
result.page_pngs.forEach((pngData, i) => {
  const pngBlob = new Blob([pngData], { type: 'image/png' });
  downloadBlob(pngBlob, `output_${i}.png`);
});
```

### import_bmfont(fnt_text, png_pages_json, source_id) → object

导入现有 BMFont。

参数：
- `fnt_text` — `.fnt` 文件文本内容
- `png_pages_json` — PNG 页面数据的 JSON 字符串，格式：`[[byte, byte, ...], ...]`（按 page id 排序）
- `source_id` — 来源标识符

返回值：
```javascript
{
  bmfont_json: "...",   // 解析后的 BmFont 结构 JSON
  glyph_count: 180      // 导入的 glyph 数量
}
```

## 输出格式

fontbake 输出标准的 AngelCode BMFont 文本格式：

### .fnt 文件

```
info face="HUN2" size=52 bold=0 italic=0 charset="" unicode=0 stretchH=100 smooth=1 aa=1 padding=4,4,4,4 spacing=-8,-8
common lineHeight=52 base=40 scaleW=1024 scaleH=1024 pages=1 packed=0
page id=0 file="HUN2.png"
chars count=180
char id=32      x=0    y=0    width=0    height=0    xoffset=-4   yoffset=0    xadvance=18   page=0    chnl=0
char id=65      x=95   y=49   width=36   height=48   xoffset=-1   yoffset=-4   xadvance=34   page=0    chnl=0
...
kernings count=0
```

### .png 文件

RGBA 格式的 atlas 纹理页。使用 DistanceField 效果时：
- RGB 通道为配置的颜色（如白色 `ffffff`）
- Alpha 通道为距离场值（128 = 边缘，>128 = 内部，<128 = 外部）

## 当前版本限制

v1 版本有以下明确限制：

| 特性 | 状态 |
|------|------|
| `render_type=0`（Java 路径） | 支持 |
| `render_type=1`（FreeType） | 不支持 |
| `render_type=2`（Native） | 不支持 |
| DistanceFieldEffect | 支持 |
| ColorEffect / ShadowEffect / GradientEffect | 不支持 |
| 文本格式 `.fnt` | 支持 |
| 二进制 BMFont v3 | 不支持 |
| 多 fallback 字体 | 支持 |
| 复杂脚本 shaping（阿拉伯语、天城文等） | 不支持 |

## 开发

```bash
# 运行所有测试
cargo test --workspace

# 只运行核心库测试
cargo test -p fontbake-core

# 构建 release 版本
cargo build --release

# 构建 WASM
wasm-pack build crates/fontbake-wasm --target web
```

