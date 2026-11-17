"""Print the NeoForge extension coverage totals from the method rows of
docs/design/mod-api/neoforge-extensions.md: `python3 tools/neoforge_extension_counts.py`."""
from collections import Counter
from pathlib import Path
import re

PAGE = Path(__file__).resolve().parents[1] / 'docs/design/mod-api/neoforge-extensions.md'


def counts(content):
    """{interface: Counter of its rows' verdicts}, in the page's section order. An `IBlockStateExtension` row
    repeats the `IBlockExtension` row of the same method, so its tally is the block's minus its own rows: that one
    is derived after every section is read, so either section may come first."""
    sections = [(name, re.findall(r'^\| `([^`]+)` \| (Covered|Partial|Missing|n/a)\b', section, re.M))
                for name, section in re.findall(r'## `(I\w+Extension)`\n(.*?)(?=\n## |\Z)', content, re.S)]
    state_rows = next((rows for name, rows in sections if name == 'IBlockStateExtension'), None)
    result = {name: Counter(verdict for _, verdict in rows) for name, rows in sections}
    if state_rows is not None:
        tally = result['IBlockExtension'].copy()
        for _, verdict in state_rows:
            tally[verdict] -= 1
        result['IBlockStateExtension'] = tally
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
