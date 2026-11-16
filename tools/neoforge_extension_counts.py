"""Print the NeoForge extension coverage totals from the method rows of
docs/design/mod-api/neoforge-extensions.md: `python3 tools/neoforge_extension_counts.py`."""
from collections import Counter
from pathlib import Path
import re

PAGE = Path(__file__).resolve().parents[1] / 'docs/design/mod-api/neoforge-extensions.md'


def counts(content):
    result = {}
    for name, section in re.findall(r'## `(I\w+Extension)`\n(.*?)(?=\n## |\Z)', content, re.S):
        rows = re.findall(r'^\| `([^`]+)` \| (Covered|Partial|Missing|n/a)\b', section, re.M)
        if name == 'IBlockStateExtension':
            tally = result['IBlockExtension'].copy()
            for _, verdict in rows:
                tally[verdict] -= 1
        else:
            tally = Counter(verdict for _, verdict in rows)
        result[name] = tally
    return result


def render(content):
    tallies = counts(content)
    table = ['| Interface | Public methods | Covered | Partial | Missing | n/a |',
             '| --- | ---: | ---: | ---: | ---: | ---: |']
    for name, tally in tallies.items():
        values = [sum(tally.values())] + [tally[v] for v in ('Covered', 'Partial', 'Missing', 'n/a')]
        table.append(f'| `{name}` | ' + ' | '.join(map(str, values)) + ' |')
    return content.replace('<!-- extension-counts -->', '\n'.join(table))


if __name__ == '__main__':
    print(render('<!-- extension-counts -->\n\n' + PAGE.read_text()).split('\n\n', 1)[0])
