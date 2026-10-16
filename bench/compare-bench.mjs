#!/usr/bin/env node
// Compare `rapidraw bench --json` results of two or more builds, in order, for example one
// per commit of a branch. For each phase it prints the median time summed over the images
// all runs share, the change from the previous run, and whether the output changed.
//
// Usage: node bench/compare-bench.mjs [label=]result.json [label=]result.json ...
// Exits 1 when a phase's output differs between two consecutive runs.
import { readFileSync } from 'node:fs';
import { basename } from 'node:path';

// Smaller differences between medians are within run-to-run noise and not shown.
const NOISE = 0.05;

const args = process.argv.slice(2);
if (args.length < 2) {
  console.error('Usage: node bench/compare-bench.mjs [label=]result.json [label=]result.json ...');
  process.exit(2);
}

const runs = args.map((arg) => {
  const split = arg.indexOf('=');
  const [label, file] = split > 0 ? [arg.slice(0, split), arg.slice(split + 1)] : [basename(arg, '.json'), arg];
  return { label, ...JSON.parse(readFileSync(file, 'utf8')) };
});

const shared = (lists) => lists.reduce((a, b) => a.filter((x) => b.includes(x)));
const images = shared(runs.map((run) => run.images.map((image) => image.image)));
if (images.length === 0) {
  console.error('The runs have no image in common.');
  process.exit(2);
}
// Phases that ran for every shared image of every run, in the order the benchmark runs them.
const phaseNames = shared(
  runs.map((run) =>
    run.phases
      .map((phase) => phase.name)
      .filter((name) => run.images.every((image) => !images.includes(image.image) || image.phases[name])),
  ),
);
const descriptions = Object.fromEntries(runs.at(-1).phases.map((phase) => [phase.name, phase.description]));

for (const run of runs.slice(1)) {
  for (const [key, value] of Object.entries(run.meta)) {
    if (JSON.stringify(value) !== JSON.stringify(runs[0].meta[key])) {
      console.warn(
        `Warning: ${key} differs: ${JSON.stringify(runs[0].meta[key])} in ${runs[0].label}, ` +
          `${JSON.stringify(value)} in ${run.label}`,
      );
    }
  }
}

// Per run and phase: the summed median time and the outputs of the shared images.
const results = runs.map((run) => {
  const byImage = Object.fromEntries(run.images.map((image) => [image.image, image.phases]));
  return Object.fromEntries(
    phaseNames.map((name) => [
      name,
      {
        ms: images.reduce((sum, image) => sum + byImage[image][name].median_ms, 0),
        outputs: images.map((image) => byImage[image][name].output),
      },
    ]),
  );
});

const formatMs = (ms) => (ms >= 100 ? ms.toFixed(0) : ms.toFixed(1));
const width = Math.max(16, ...runs.map((run) => run.label.length + 2));
const phaseWidth = Math.max(...phaseNames.map((name) => name.length)) + 2;

console.log(`Median time in ms, summed over ${images.length} image(s); change from the previous column.`);
console.log(`* = output differs from the previous column.\n`);
console.log('phase'.padEnd(phaseWidth) + runs.map((run) => run.label.padStart(width)).join(''));

const changes = [];
for (const name of phaseNames) {
  const cells = results.map((result, i) => {
    const current = result[name];
    if (!current) return '-'.padStart(width);
    const previous = i > 0 ? results[i - 1][name] : undefined;
    let cell = formatMs(current.ms);
    if (previous) {
      const change = current.ms / previous.ms - 1;
      if (Math.abs(change) >= NOISE) cell += ` ${change > 0 ? '+' : ''}${(change * 100).toFixed(0)}%`;
      const differing = images.filter((_, k) => current.outputs[k] !== previous.outputs[k]);
      if (differing.length > 0) {
        cell += '*';
        changes.push(`${name}: ${runs[i - 1].label} -> ${runs[i].label} (${differing.join(', ')})`);
      }
    }
    return cell.padStart(width);
  });
  console.log(name.padEnd(phaseWidth) + cells.join(''));
}

const first = results[0];
const last = results.at(-1);
console.log(`\nSpeedup of ${runs.at(-1).label} over ${runs[0].label}:`);
for (const name of phaseNames) {
  if (first[name] && last[name]) {
    const speedup = first[name].ms / last[name].ms;
    console.log(`  ${name.padEnd(phaseWidth)}${speedup.toFixed(2).padStart(6)}x   ${descriptions[name] ?? ''}`);
  }
}

if (changes.length > 0) {
  console.log('\nOutput changed:');
  for (const change of changes) console.log(`  ${change}`);
  process.exit(1);
}
console.log('\nAll outputs are identical in every run.');
