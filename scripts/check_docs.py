"""Check local Markdown links in the project's maintained documentation."""
from pathlib import Path
import re
import sys
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[1]
FILES = [ROOT / 'README.md']
for folder in ['docs', 'performance', 'proof-of-work']:
    FILES.extend(p for p in (ROOT / folder).rglob('*.md') if 'target' not in p.parts)


def without_code(text):
    return re.sub(r'^```[^\n]*\n.*?^```[^\n]*$', '', text, flags=re.M | re.S)


def anchors(text):
    result, seen = set(), {}
    for title in re.findall(r'^#{1,6}\s+(.+?)\s*#*$', without_code(text), re.M):
        title = title.replace('`', '').lower()
        base = ''.join(c for c in title if c.isalnum() or c in ' _-').replace(' ', '-')
        count = seen.get(base, 0)
        seen[base] = count + 1
        result.add(base + (f'-{count}' if count else ''))
    return result


errors = []
for source in sorted(FILES):
    text = without_code(source.read_text())
    for match in re.finditer(r'(?<!!)\[[^\]\n]+\]\(([^\s)]+)\)', text):
        link = match[1].strip('<>')
        url = urlsplit(link)
        if url.scheme or url.netloc:
            continue
        path = (source.parent / unquote(url.path)).resolve() if url.path else source
        reason = None
        if not path.exists():
            reason = 'missing target'
        elif url.fragment and path.suffix == '.md' and unquote(url.fragment) not in anchors(path.read_text()):
            reason = 'missing heading'
        if reason:
            line = text.count('\n', 0, match.start()) + 1
            errors.append(f'{source.relative_to(ROOT)}:{line}: {reason}: {link}')
if errors:
    print('\n'.join(errors), file=sys.stderr)
    sys.exit(1)
print(f'Local links verified in {len(FILES)} Markdown documents.')
