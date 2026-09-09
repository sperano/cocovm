"""Regenerate tables from the checked-in display capture JSON files."""
import json
import statistics as stats
from pathlib import Path

RESULTS = Path(__file__).resolve().parent
MIB = 1024 ** 2

def measured(run):
    m, r = run['metrics'], run['resources']
    seconds = m['scenario']['duration_seconds']
    stages = m['stages']
    return {
        'CPU %': r['cpu_percent_one_core'],
        'RSS MiB': r['peak_sampled_rss_bytes'] / MIB,
        'updates/s': stages['vm_ui_update']['count'] / seconds,
        'uploads/s': m['texture_enqueue_cpu']['count'] / seconds,
        'upload MiB/s': m['texture_enqueue_cpu']['bytes'] / seconds / MIB,
        'allocations/s': m['allocations']['count'] / seconds,
        'allocated MiB/s': m['allocations']['requested_bytes'] / seconds / MIB,
        'VM p95 ms': stages['vm_ui_update']['p95_ns'] / 1_000_000,
        'conversion p95 ms': stages['display_conversion']['p95_ns'] / 1_000_000,
        'fields/s': m['scenario']['fields_run'] / seconds,
        'missing audio frames': m['audio']['missing_frames'],
        'overflow frames': m['audio']['overflow_frames'],
        'unfocused updates': m['scenario']['unfocused_updates'],
        'operations': m['scenario']['operations'],
    }

def datasets():
    for name in ('before-native', 'after-native', 'before-tv', 'after-tv'):
        path = RESULTS / f'{name}.json'
        if path.exists():
            runs = json.loads(path.read_text())['runs']
            followup = RESULTS / 'before-followup.json'
            if name == 'before-native' and followup.exists():
                runs = [r for r in runs if r['run'] != 'basic-idle-0']
                runs += [r for r in json.loads(followup.read_text())['runs']
                         if r['metrics']['scenario']['name'] == 'basic-idle']
            yield name, runs

def summary():
    result = {}
    for name, runs in datasets():
        for run in runs:
            scenario = run['metrics']['scenario']['name']
            result.setdefault((name, scenario), []).append(measured(run))
    return result

def cell(values):
    return f'{stats.median(values):.2f} [{min(values):.2f}–{max(values):.2f}]'

def table(data, metrics):
    rows = ['| Capture / scenario | ' + ' | '.join(metrics) + ' |',
            '|---|' + '---:|' * len(metrics)]
    for (name, scenario), runs in data.items():
        rows.append(f'| {name} / {scenario} | ' + ' | '.join(
            cell([run[key] for run in runs]) for key in metrics) + ' |')
    return '\n'.join(rows)

if __name__ == '__main__':
    data = summary()
    sections = [
        '# Display measurements',
        'Values are medians with minimum–maximum ranges across three fresh-process captures. '
        'Rates use each run\'s actual measurement duration. MiB means 1,048,576 bytes. '
        'CPU percentage uses one core = 100%. Four-VM rates sum all VMs.',
        table(data, ['updates/s', 'uploads/s', 'upload MiB/s', 'allocated MiB/s']),
        table(data, ['CPU %', 'RSS MiB', 'VM p95 ms', 'conversion p95 ms']),
        table(data, ['fields/s', 'missing audio frames', 'overflow frames', 'unfocused updates', 'operations']),
        'Texture metrics count CPU enqueue requests. Conversion timings cover cache misses only; '
        'cache comparison is included in VM UI time. Snapshot and lifecycle audio measurements '
        'include intentional resets and silence. See [methods](METHODS.md) for other boundaries.',
    ]
    (RESULTS / 'MEASUREMENTS.md').write_text('\n\n'.join(sections) + '\n')
    for key, runs in data.items():
        print(key, {metric: round(stats.median([r[metric] for r in runs]), 3)
                    for metric in ['uploads/s','allocated MiB/s','CPU %','fields/s']})
