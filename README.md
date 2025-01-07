# A github action for running benchmarks on our infra

## Usage

1. Create a new repo to hold the benchmarking data with `main` as default branch.
2. Copy the gh-pages branch from <https://github.com/trifectatechfoundation/zlib-rs-bench/> into the bench repo and adapt as relevant.
3. Create a deploy key for the bench repo and add it as BENCH_DATA_DEPLOY_KEY secret to the repo which will run the benchmarks.
4. Create a .json file specifying all benchmarks to run. For example:

```json
{
    "blogpost-compress-rs": [
        "./blogpost-compress 0 rs silesia-small.tar",
        "./blogpost-compress 1 rs silesia-small.tar",
        "./blogpost-compress 2 rs silesia-small.tar"
    ],
    "blogpost-compress-ng": [
        "./blogpost-compress 0 ng silesia-small.tar",
        "./blogpost-compress 1 ng silesia-small.tar",
        "./blogpost-compress 2 ng silesia-small.tar"
    ]
}
```

5. Add a new benchmark workflow which runs the following step on our self-hosted x86_64 and arm64 runners on every push and optionally when manually dispatched (adapt as necessary):

```yaml
- name: Benchmark
  uses: trifectatechfoundation/benchmarker-action@main
  with:
    deploy-key: "${{ secrets.BENCH_DATA_DEPLOY_KEY }}"
    bench-repo: "git@github.com:trifectatechfoundation/zlib-rs-bench.git"
    metric-key: "${{ matrix.name }}"
    benchmarks: "zlib_benchmarks.json"
```
