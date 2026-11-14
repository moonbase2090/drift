#!/usr/bin/env python3
"""Render DRIFT_FRAME records from a real render_once test run as an animated GIF."""
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile
from xml.sax.saxutils import escape


def rgb(values):
    return '#' + ''.join(f'{v:02x}' for v in values)


def frame_svg(frame):
    colors = frame['colors']
    rows, columns = len(colors), len(colors[0])
    cell_width, cell_height = 8, 16
    base = tuple(map(int, re.findall(r'\d+', colors[0][0])))[3:]
    assert len(base) == 3
    out = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{columns * cell_width}" height="{rows * cell_height}">']
    for y, line in enumerate(frame['text'].splitlines()):
        for x, character in enumerate(line):
            if x >= columns:
                break
            values = tuple(map(int, re.findall(r'\d+', colors[y][x])))
            if len(values) == 3:
                values += base
            assert len(values) == 6
            out.append(f'<rect x="{x * cell_width}" y="{y * cell_height}" width="8" height="16" fill="{rgb(values[3:])}"/>')
            if character != ' ':
                out.append(f'<text x="{x * cell_width}" y="{y * cell_height + 13}" font-family="Noto Sans Mono" font-size="13" fill="{rgb(values[:3])}">{escape(character)}</text>')
    out.append('</svg>')
    return ''.join(out)


def main():
    source, output = map(Path, sys.argv[1:])
    frames = [json.loads(line.partition('DRIFT_FRAME ')[2])
              for line in source.read_text().splitlines() if 'DRIFT_FRAME ' in line]
    if not frames:
        raise SystemExit('No DRIFT_FRAME records; run the capture test with DRIFT_CAPTURE=1 and --nocapture.')
    with tempfile.TemporaryDirectory(prefix='drift-tui-proof-') as scratch:
        paths = []
        for index, frame in enumerate(frames):
            path = Path(scratch) / f'{index:03}.svg'
            path.write_text(frame_svg(frame))
            paths.append(str(path))
        subprocess.run(['magick', '-delay', '180', *paths, '-loop', '0', str(output)], check=True)
    print(f'{len(frames)} real rendered frames: {output}')


if __name__ == '__main__':
    main()
