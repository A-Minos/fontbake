#!/usr/bin/env python3
import argparse
import json
import math
import re
from pathlib import Path

from PIL import Image, ImageChops, ImageOps, ImageStat, ImageDraw

KV_RE = re.compile(r'(\w+)=((?:"[^"]*")|\S+)')


def parse_kv(line: str) -> dict[str, str]:
    return {k: v.strip('"') for k, v in KV_RE.findall(line)}


def parse_fnt(path: Path) -> dict:
    info = None
    common = None
    pages = {}
    chars = {}

    for raw_line in path.read_text(encoding='utf-8').splitlines():
        line = raw_line.strip()
        if not line:
            continue
        if line.startswith('info '):
            info = parse_kv(line)
        elif line.startswith('common '):
            common = parse_kv(line)
        elif line.startswith('page '):
            item = parse_kv(line)
            pages[int(item['id'])] = item['file']
        elif line.startswith('char '):
            item = parse_kv(line)
            chars[int(item['id'])] = {k: int(v) for k, v in item.items() if k != 'id'}

    if info is None or common is None:
        raise ValueError(f'invalid fnt file: {path}')

    return {
        'path': path,
        'info': info,
        'common': {k: int(v) if v.lstrip('-').isdigit() else v for k, v in common.items()},
        'pages': pages,
        'chars': chars,
    }


def compare_fnt(lhs: dict, rhs: dict) -> dict:
    lhs_ids = set(lhs['chars'])
    rhs_ids = set(rhs['chars'])
    shared_ids = sorted(lhs_ids & rhs_ids)

    metric_names = ['width', 'height', 'xoffset', 'yoffset', 'xadvance']
    metric_diffs = {}
    for name in metric_names:
        diffs = []
        samples = []
        for cp in shared_ids:
            lv = lhs['chars'][cp][name]
            rv = rhs['chars'][cp][name]
            delta = lv - rv
            if delta != 0:
                diffs.append(abs(delta))
                if len(samples) < 10:
                    samples.append({'id': cp, 'lhs': lv, 'rhs': rv, 'delta': delta})
        metric_diffs[name] = {
            'diff_count': len(diffs),
            'max_abs_diff': max(diffs) if diffs else 0,
            'samples': samples,
        }

    common_keys = ['lineHeight', 'base', 'scaleW', 'scaleH', 'pages']
    common_diff = {
        key: {'lhs': lhs['common'].get(key), 'rhs': rhs['common'].get(key)}
        for key in common_keys
        if lhs['common'].get(key) != rhs['common'].get(key)
    }

    info_keys = ['face', 'size', 'padding', 'spacing']
    info_diff = {
        key: {'lhs': lhs['info'].get(key), 'rhs': rhs['info'].get(key)}
        for key in info_keys
        if lhs['info'].get(key) != rhs['info'].get(key)
    }

    return {
        'char_count': {'lhs': len(lhs_ids), 'rhs': len(rhs_ids)},
        'missing_chars': {
            'only_lhs': sorted(lhs_ids - rhs_ids),
            'only_rhs': sorted(rhs_ids - lhs_ids),
        },
        'common_diff': common_diff,
        'info_diff': info_diff,
        'metric_diffs': metric_diffs,
    }


def alpha_image(path: Path):
    image = Image.open(path).convert('RGBA')
    return image.getchannel('A')


def crop_alpha(alpha, x: int, y: int, width: int, height: int):
    if width <= 0 or height <= 0:
        return []
    return list(alpha.crop((x, y, x + width, y + height)).getdata())


def crop_alpha_image(alpha, x: int, y: int, width: int, height: int):
    if width <= 0 or height <= 0:
        return Image.new('L', (1, 1), 0)
    return alpha.crop((x, y, x + width, y + height))


def compare_alpha_vectors(lhs_alpha, rhs_alpha) -> dict:
    compare_len = min(len(lhs_alpha), len(rhs_alpha))

    total_abs = 0
    total_sq = 0
    diff_pixels = 0
    max_abs = 0

    for i in range(compare_len):
        diff = lhs_alpha[i] - rhs_alpha[i]
        abs_diff = abs(diff)
        total_abs += abs_diff
        total_sq += diff * diff
        if abs_diff != 0:
            diff_pixels += 1
            if abs_diff > max_abs:
                max_abs = abs_diff

    if compare_len == 0:
        mean_abs = 0.0
        rmse = 0.0
        equal_ratio = 1.0
    else:
        mean_abs = total_abs / compare_len
        rmse = math.sqrt(total_sq / compare_len)
        equal_ratio = (compare_len - diff_pixels) / compare_len

    return {
        'sample_len': compare_len,
        'mean_abs_diff': mean_abs,
        'rmse': rmse,
        'max_abs_diff': max_abs,
        'diff_pixels': diff_pixels,
        'equal_ratio': equal_ratio,
    }


def compare_png(lhs_path: Path, rhs_path: Path) -> dict:
    lhs_image = alpha_image(lhs_path)
    rhs_image = alpha_image(rhs_path)
    lhs_alpha = list(lhs_image.getdata())
    rhs_alpha = list(rhs_image.getdata())

    width = min(lhs_image.size[0], rhs_image.size[0])
    height = min(lhs_image.size[1], rhs_image.size[1])
    compare_len = width * height

    metrics = compare_alpha_vectors(lhs_alpha[:compare_len], rhs_alpha[:compare_len])
    return {
        'lhs_size': list(lhs_image.size),
        'rhs_size': list(rhs_image.size),
        'compare_region': [width, height],
        'mean_abs_diff': metrics['mean_abs_diff'],
        'rmse': metrics['rmse'],
        'max_abs_diff': metrics['max_abs_diff'],
        'diff_pixels': metrics['diff_pixels'],
        'equal_ratio': metrics['equal_ratio'],
    }


def compare_glyph_alpha(lhs: dict, rhs: dict, sample_limit: int = 12) -> dict:
    lhs_dir = lhs['path'].parent
    rhs_dir = rhs['path'].parent
    lhs_pages = {page_id: alpha_image(lhs_dir / page_file) for page_id, page_file in lhs['pages'].items()}
    rhs_pages = {page_id: alpha_image(rhs_dir / page_file) for page_id, page_file in rhs['pages'].items()}

    shared_ids = sorted(set(lhs['chars']) & set(rhs['chars']))
    per_glyph = []
    total_abs = 0.0
    total_sq = 0.0
    total_pixels = 0
    total_diff_pixels = 0
    worst_max_abs = 0

    for cp in shared_ids:
        lc = lhs['chars'][cp]
        rc = rhs['chars'][cp]
        if lc['width'] <= 0 or lc['height'] <= 0 or rc['width'] <= 0 or rc['height'] <= 0:
            continue
        lw = min(lc['width'], rc['width'])
        lh = min(lc['height'], rc['height'])
        lhs_crop = crop_alpha(lhs_pages[lc['page']], lc['x'], lc['y'], lw, lh)
        rhs_crop = crop_alpha(rhs_pages[rc['page']], rc['x'], rc['y'], lw, lh)
        metrics = compare_alpha_vectors(lhs_crop, rhs_crop)
        pixels = metrics['sample_len']
        total_abs += metrics['mean_abs_diff'] * pixels
        total_sq += (metrics['rmse'] ** 2) * pixels
        total_pixels += pixels
        total_diff_pixels += metrics['diff_pixels']
        worst_max_abs = max(worst_max_abs, metrics['max_abs_diff'])
        per_glyph.append({
            'id': cp,
            'char': chr(cp),
            'lhs_rect': {'x': lc['x'], 'y': lc['y'], 'width': lc['width'], 'height': lc['height']},
            'rhs_rect': {'x': rc['x'], 'y': rc['y'], 'width': rc['width'], 'height': rc['height']},
            'width': lw,
            'height': lh,
            **metrics,
        })

    per_glyph_sorted = sorted(
        per_glyph,
        key=lambda item: (item['mean_abs_diff'], item['rmse'], item['max_abs_diff']),
        reverse=True,
    )

    if total_pixels == 0:
        summary = {
            'glyphs_compared': 0,
            'mean_abs_diff': 0.0,
            'rmse': 0.0,
            'max_abs_diff': 0,
            'diff_pixels': 0,
            'equal_ratio': 1.0,
        }
    else:
        summary = {
            'glyphs_compared': len(per_glyph),
            'mean_abs_diff': total_abs / total_pixels,
            'rmse': math.sqrt(total_sq / total_pixels),
            'max_abs_diff': worst_max_abs,
            'diff_pixels': total_diff_pixels,
            'equal_ratio': (total_pixels - total_diff_pixels) / total_pixels,
        }

    return {
        **summary,
        'top_diff_glyphs': per_glyph_sorted[:sample_limit],
    }


def render_glyph_comparison(output_dir: Path, lhs: dict, rhs: dict, glyph_entries: list[dict]):
    output_dir.mkdir(parents=True, exist_ok=True)
    lhs_dir = lhs['path'].parent
    rhs_dir = rhs['path'].parent
    lhs_pages = {page_id: alpha_image(lhs_dir / page_file) for page_id, page_file in lhs['pages'].items()}
    rhs_pages = {page_id: alpha_image(rhs_dir / page_file) for page_id, page_file in rhs['pages'].items()}

    for entry in glyph_entries:
        cp = entry['id']
        lc = lhs['chars'][cp]
        rc = rhs['chars'][cp]
        lhs_img = crop_alpha_image(lhs_pages[lc['page']], lc['x'], lc['y'], lc['width'], lc['height'])
        rhs_img = crop_alpha_image(rhs_pages[rc['page']], rc['x'], rc['y'], rc['width'], rc['height'])

        target_w = max(lhs_img.width, rhs_img.width)
        target_h = max(lhs_img.height, rhs_img.height)
        lhs_pad = Image.new('L', (target_w, target_h), 0)
        rhs_pad = Image.new('L', (target_w, target_h), 0)
        lhs_pad.paste(lhs_img, (0, 0))
        rhs_pad.paste(rhs_img, (0, 0))

        diff = ImageChops.difference(lhs_pad, rhs_pad)
        diff_rgb = Image.merge('RGB', (diff, Image.new('L', diff.size, 0), rhs_pad))
        lhs_rgb = Image.merge('RGB', (lhs_pad, lhs_pad, lhs_pad))
        rhs_rgb = Image.merge('RGB', (rhs_pad, rhs_pad, rhs_pad))

        scale = 4
        lhs_big = lhs_rgb.resize((target_w * scale, target_h * scale), Image.Resampling.NEAREST)
        rhs_big = rhs_rgb.resize((target_w * scale, target_h * scale), Image.Resampling.NEAREST)
        diff_big = diff_rgb.resize((target_w * scale, target_h * scale), Image.Resampling.NEAREST)

        panel_gap = 12
        label_h = 24
        canvas_w = lhs_big.width * 3 + panel_gap * 4
        canvas_h = lhs_big.height + label_h + panel_gap * 2
        canvas = Image.new('RGB', (canvas_w, canvas_h), (24, 24, 24))
        draw = ImageDraw.Draw(canvas)

        positions = [panel_gap, panel_gap * 2 + lhs_big.width, panel_gap * 3 + lhs_big.width * 2]
        labels = ['fontbake', 'hiero', 'diff']
        images = [lhs_big, rhs_big, diff_big]
        for x, label, img in zip(positions, labels, images):
            draw.text((x, 4), label, fill=(220, 220, 220))
            canvas.paste(img, (x, label_h))

        char_text = entry['char'] if entry['char'].isprintable() and entry['char'] != ' ' else f'U+{cp:04X}'
        safe_name = f'{cp:04X}'
        out_path = output_dir / f'{safe_name}.png'
        canvas.save(out_path)
        entry['comparison_image'] = str(out_path)
        entry['label'] = char_text


def main() -> int:
    repo = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(description='Compare fontbake output against Hiero baseline.')
    parser.add_argument('--fontbake-fnt', type=Path, default=repo / '.fnt')
    parser.add_argument('--fontbake-png', type=Path, default=repo / '.png')
    parser.add_argument('--hiero-fnt', type=Path, default=repo / 'target/hiero-compare/font.fnt')
    parser.add_argument('--hiero-png', type=Path, default=repo / 'target/hiero-compare/font.png')
    parser.add_argument('--export-glyph-dir', type=Path)
    args = parser.parse_args()

    lhs_fnt = parse_fnt(args.fontbake_fnt)
    rhs_fnt = parse_fnt(args.hiero_fnt)
    glyph_alpha = compare_glyph_alpha(lhs_fnt, rhs_fnt)

    if args.export_glyph_dir is not None:
        render_glyph_comparison(args.export_glyph_dir, lhs_fnt, rhs_fnt, glyph_alpha['top_diff_glyphs'])

    result = {
        'fontbake_fnt': str(args.fontbake_fnt),
        'fontbake_png': str(args.fontbake_png),
        'hiero_fnt': str(args.hiero_fnt),
        'hiero_png': str(args.hiero_png),
        'fnt': compare_fnt(lhs_fnt, rhs_fnt),
        'png_alpha': compare_png(args.fontbake_png, args.hiero_png),
        'glyph_alpha': glyph_alpha,
    }

    print(json.dumps(result, ensure_ascii=False, indent=2))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
