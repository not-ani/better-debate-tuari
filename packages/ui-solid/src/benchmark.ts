import { buildDebatifyTagTreeRows } from "./lib/remoteSearch/treeRows";
import type { DebatifyTagHit } from "./lib/remoteSearch/types";

const sampleHits: DebatifyTagHit[] = Array.from({ length: 500 }, (_, index) => ({
  id: `hit-${index}`,
  tag: `Tag ${index}`,
  citation: `Source ${index}`,
  richHtml: `<p>Body ${index}</p>`,
  plainText: `Body ${index}`,
  copyText: `Tag ${index}\n\nSource ${index}\n\nBody ${index}`,
  paragraphXml: [`<w:p>${index}</w:p>`],
  sourcePath: `https://api.debatify.app/search?q=test#result-${index + 1}`,
}));

type BenchmarkStats = {
  iterations: number;
  minMs: number;
  p50Ms: number;
  p95Ms: number;
  maxMs: number;
  meanMs: number;
};

const percentile = (values: number[], ratio: number) => {
  if (values.length === 0) return 0;
  const bounded = Math.max(0, Math.min(1, ratio));
  const index = Math.min(values.length - 1, Math.round((values.length - 1) * bounded));
  return values[index] ?? 0;
};

const runBenchmark = (name: string, fn: () => void, iterations = 3_000): BenchmarkStats => {
  for (let warmup = 0; warmup < 300; warmup += 1) {
    fn();
  }

  const samples: number[] = [];
  for (let iteration = 0; iteration < iterations; iteration += 1) {
    const started = performance.now();
    fn();
    samples.push(performance.now() - started);
  }

  samples.sort((left, right) => left - right);
  const meanMs = samples.reduce((sum, value) => sum + value, 0) / samples.length;
  const stats: BenchmarkStats = {
    iterations,
    minMs: samples[0] ?? 0,
    p50Ms: percentile(samples, 0.5),
    p95Ms: percentile(samples, 0.95),
    maxMs: samples[samples.length - 1] ?? 0,
    meanMs,
  };

  console.log(
    `${name}: p50=${stats.p50Ms.toFixed(4)}ms p95=${stats.p95Ms.toFixed(4)}ms mean=${stats.meanMs.toFixed(4)}ms`,
  );
  return stats;
};

console.log("UI benchmark input size:", sampleHits.length, "remote tag hits");
runBenchmark("buildDebatifyTagTreeRows (collapsed)", () => {
  buildDebatifyTagTreeRows(sampleHits, false);
});
runBenchmark("buildDebatifyTagTreeRows (expanded)", () => {
  buildDebatifyTagTreeRows(sampleHits, true);
});
