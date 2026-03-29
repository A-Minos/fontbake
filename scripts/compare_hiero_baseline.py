#!/usr/bin/env python3
import argparse
import json
import math
import re
from pathlib import Path

from PIL import Image

KV_RE = re.compile(r'(\w+)=((?:"[^"]*")|\S+)')


def parse_kv(line: str) -> dict[str, str]:
    return {k: v.strip('"') for k, v in KV_RE.findall(line)}


def parse_fnt(path: Path) -> dict:
    info = None
    common = None
    chars = {}

    for raw_line in path.read_text(encoding='utf-8').splitlines():
        line = raw_line.strip()
        if not line:
            continue
        if line.startswith('info '):
            info = parse_kv(line)
        elif line.startswith('common '):
            common = parse_kv(line)
        elif line.startswith('char '):
            item = parse_kv(line)
            chars[int(item['id'])] = {k: int(v) for k, v in item.items() if k != 'id'}

    if info is None or common is None:
        raise ValueError(f'invalid fnt file: {path}')

    return {
        'info': info,
        'common': {k: int(v) if v.lstrip('-').isdigit() else v for k, v in common.items()},
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


def alpha_channel(path: Path):
    image = Image.open(path).convert('RGBA')
    alpha = image.getchannel('A')
    return image.size, list(alpha.getdata())


def compare_png(lhs_path: Path, rhs_path: Path) -> dict:
    lhs_size, lhs_alpha = alpha_channel(lhs_path)
    rhs_size, rhs_alpha = alpha_channel(rhs_path)

    width = min(lhs_size[0], rhs_size[0])
    height = min(lhs_size[1], rhs_size[1])
    compare_len = width * height

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
        'lhs_size': list(lhs_size),
        'rhs_size': list(rhs_size),
        'compare_region': [width, height],
        'mean_abs_diff': mean_abs,
        'rmse': rmse,
        'max_abs_diff': max_abs,
        'diff_pixels': diff_pixels,
        'equal_ratio': equal_ratio,
    }


def main() -> int:
    repo = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(description='Compare fontbake output against Hiero baseline.')
    parser.add_argument('--fontbake-fnt', type=Path, default=repo / '.fnt')
    parser.add_argument('--fontbake-png', type=Path, default=repo / '.png')
    parser.add_argument('--hiero-fnt', type=Path, default=repo / 'target/hiero-compare/font.fnt')
    parser.add_argument('--hiero-png', type=Path, default=repo / 'target/hiero-compare/font.png')
    args = parser.parse_args()

    lhs_fnt = parse_fnt(args.fontbake_fnt)
    rhs_fnt = parse_fnt(args.hiero_fnt)

    result = {
        'fontbake_fnt': str(args.fontbake_fnt),
        'fontbake_png': str(args.fontbake_png),
        'hiero_fnt': str(args.hiero_fnt),
        'hiero_png': str(args.hiero_png),
        'fnt': compare_fnt(lhs_fnt, rhs_fnt),
        'png_alpha': compare_png(args.fontbake_png, args.hiero_png),
    }

    print(json.dumps(result, ensure_ascii=False, indent=2))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
