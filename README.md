# fontbake

一个用 Rust 编写的字体构建工具，目标是尽量对齐 libGDX Hiero 的 Java 路径输出，同时提供 CLI 和 WASM 两种使用方式。

## 当前状态

- 支持解析 Hiero `.hiero` 配置并生成文本 BMFont（`.fnt` + `.png`）
- 只支持 `render_type=0`（Java 路径）
- 只支持 `DistanceFieldEffect`
- `fontbake-core` 维持纯 bytes-in / bytes-out，不直接访问文件系统
- 支持多 fallback 字体（fontbake 扩展功能）

## 安装与构建

### 从源码编译

需要 Rust 1.85+。

```bash
git clone <repo-url> fontbake
cd fontbake
cargo build --release
```

CLI 产物位于 `target/release/fontbake`。

### 构建 WASM

```bash
cargo install wasm-pack
wasm-pack build crates/fontbake-wasm --target web
```

产物位于 `crates/fontbake-wasm/pkg/`。

## 项目结构

```
fontbake/
├── Cargo.toml
└── crates/
    ├── fontbake-core/       # 核心库
    │   ├── config/          # .hiero 解析
    │   ├── model/           # 核心数据模型
    │   ├── source/          # 输入源（TTF/OTF/BMFont）
    │   ├── effect/          # DistanceFieldEffect
    │   ├── export/          # 文本 BMFont / PNG 导出
    │   ├── pack/            # glyph page packer
    │   ├── pipeline/        # build/import/merge 主流程
    │   └── raster/          # Java 路径光栅化
    ├── fontbake-cli/        # CLI 工具
    └── fontbake-wasm/       # WASM 绑定
```

## `.hiero` 配置支持

### 配置示例

```ini
font.name=MyFont
font.size=52
font.bold=false
font.italic=false
font.gamma=1.8
font.mono=false

font2.file=/path/to/primary.ttf


pad.top=4
pad.right=4
pad.bottom=4
pad.left=4
pad.advance.x=-8
pad.advance.y=-8


glyph.page.width=1024
glyph.page.height=1024
glyph.text=ABCDEFG0123456789

render_type=0

fallback.font.0=/path/to/fallback1.otf
fallback.font.1=/path/to/fallback2.ttf

effect.class=com.badlogic.gdx.tools.hiero.unicodefont.effects.DistanceFieldEffect
effect.Color=ffffff
effect.Scale=32
effect.Spread=3.5
```

### 字段支持状态

| 字段 | 状态 | 说明 |
|---|---|---|
| `font2.file` | 支持 | 主字体文件来源 |
| `fallback.font.N` | 支持 | fontbake 扩展，按顺序回退，从 0 开始连续编号 |
| `glyph.text` | 支持 | 按 Hiero 语义保留原始字符序列，不 trim 空格 |
| `glyph.page.width/height` | 支持 | Atlas 页尺寸 |
| `pad.*` | 支持 | padding/advance 调整 |
| `render_type` | 仅支持 0 | 仅支持 `0`（Java 路径），其他值会报错 |
| `DistanceFieldEffect` | 支持 | 当前唯一支持的 effect |
| 其它 effect | 不支持 | 如 Shadow / Gradient / Outline |
| `font2.use` | 不支持 | parser 忽略此字段 |
| `glyph.native.rendering` | 不支持 | parser 忽略此字段 |

## CLI 使用

fontbake 提供四个子命令：

### build

从 `.hiero` 配置构建字体：

```bash
fontbake build <配置文件> -o <输出目录>
```

示例：

```bash
fontbake build config.hiero -o output/
```

输出：
- `output/<fontname>.fnt`
- `output/<fontname>.png`
- 如果多页，则继续输出 `output/<fontname>_1.png`、`output/<fontname>_2.png` ...

### import

导入现有文本 BMFont：

```bash
fontbake import <fnt文件> -o <输出目录>
```

示例：

```bash
fontbake import some-font.fnt -o reimported/
```

说明：
- 仅支持文本格式 BMFont
- bitmap glyph 像素直通，不重新光栅化
- 保留原始 `xoffset` / `yoffset` / `xadvance` / kerning

### merge

合并多个 BMFont 源：

```bash
fontbake merge <fnt文件...> -n <名称> --page-width 1024 --page-height 1024 -o <输出目录>
```

示例：

```bash
fontbake merge latin.fnt cjk.fnt -n combined -o output/
```

规则：
- 按输入顺序决定优先级
- 同一 codepoint 冲突时，第一个源胜出
- 所有 glyph 会重新打包到新 atlas

### inspect

检查 `.hiero` 或 `.fnt` 的解析结果：

```bash
fontbake inspect <文件>
```

示例：

```bash
fontbake inspect config.hiero
fontbake inspect some-font.fnt
```

可以配合 `jq`：

```bash
fontbake inspect config.hiero | jq '.fallback_font_paths'
fontbake inspect some-font.fnt | jq '.chars | length'
```

## WASM API

`fontbake-wasm` 暴露五个接口：

- `parse_hiero(config_text) -> string`
- `parse_bmfont(fnt_text) -> string`
- `build_font(config_text, primary_font, fallback_fonts_json) -> object`
- `import_bmfont(fnt_text, png_pages_json, source_id) -> object`
- `merge_fonts(glyph_sets_json, merge_config_json) -> object`

### parse_hiero

```javascript
import init, { parse_hiero } from './fontbake_wasm.js';

await init();
const specJson = parse_hiero(configText);
const spec = JSON.parse(specJson);
```

### parse_bmfont

```javascript
const bmfontJson = parse_bmfont(fntText);
const bmfont = JSON.parse(bmfontJson);
```

### build_font

```javascript
const fallbackJson = JSON.stringify([
  { name: 'fallback.otf', data: Array.from(fallbackFontBytes) }
]);

const result = build_font(configText, primaryFontBytes, fallbackJson);

console.log(result.fnt_text);
console.log(result.page_pngs);
console.log(result.glyph_count);
```

返回对象：

```javascript
{
  fnt_text: 'info face=...',
  page_pngs: [Uint8Array, ...],
  glyph_count: 180,
}
```

### import_bmfont

```javascript
const result = import_bmfont(fntText, pngPagesJson, 'source-id');
console.log(result.bmfont_json);
console.log(result.glyph_count);
```

### merge_fonts

```javascript
const glyphSetsJson = JSON.stringify([glyphSetA, glyphSetB]);
const mergeConfigJson = JSON.stringify({
  face: 'combined',
  font_size: 52,
  line_height: 57,
  base: 38,
  page_width: 1024,
  page_height: 1024,
  padding: [4, 4, 4, 4],
  spacing: [-8, -8],
});

const result = merge_fonts(glyphSetsJson, mergeConfigJson);

console.log(result.fnt_text);
console.log(result.page_pngs);
console.log(result.glyph_count);
```

按输入顺序决定优先级，同一 codepoint 冲突时第一个源胜出。

## 输出格式

### `.fnt`

输出为 AngelCode BMFont 文本格式：

```text
info face="MyFont" size=52 bold=0 italic=0 charset="" unicode=0 stretchH=100 smooth=1 aa=1 padding=4,4,4,4 spacing=-8,-8
common lineHeight=57 base=38 scaleW=512 scaleH=512 pages=1 packed=0
page id=0 file="MyFont.png"
chars count=180
char id=32      x=0    y=0    width=0    height=0    xoffset=-4   yoffset=0    xadvance=18   page=0    chnl=0
...
kernings count=0
```

### `.png`

输出 atlas 为 RGBA PNG：

- RGB 通道为 effect 配置的颜色（如 `ffffff`）
- Alpha 通道为距离场值
  - `128` = 边缘
  - `>128` = 内部
  - `<128` = 外部

## 开发

```bash
# 运行 core 测试
cargo test -p fontbake-core

# 运行整个 workspace 测试
cargo test --workspace

# 构建 CLI
cargo build -p fontbake-cli --release

# 构建 WASM
wasm-pack build crates/fontbake-wasm --target web
```

## 当前限制

| 特性 | 状态 |
|---|---|
| `render_type=0`（Java 路径） | 支持 |
| `render_type=1`（FreeType） | 不支持 |
| `render_type=2`（Native） | 不支持 |
| DistanceFieldEffect | 支持 |
| 其它 Hiero effect | 不支持 |
| 文本格式 `.fnt` | 支持 |
| 二进制 BMFont v3 | 不支持 |
| 多 fallback 字体 | 支持（fontbake 扩展） |
| 复杂脚本 shaping（阿拉伯语、天城文等） | 不支持 |
