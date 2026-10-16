# Image Pipeline Benchmark (headless)

`rapidraw bench` times the internal functions behind the editor's image processing on real
images, without opening a window. Run it with two builds, or with one build per commit, to see
which change made a step faster or slower and whether it changed the output.

```bash
rapidraw bench photo1.ARW photo2.ARW --json bench/out/run.json
```

| Option           | Description                                     | Default    |
| ---------------- | ----------------------------------------------- | ---------- |
| `--iters <n>`    | Iterations of each phase                        | 5          |
| `--phases <a,b>` | Run only these phases (comma-separated)         | all phases |
| `--json <file>`  | Write the results for `bench/compare-bench.mjs` | -          |

Each image is processed with the edits in its `.rrdata` sidecar, if it has one, and with your
saved app settings. The benchmark prints the GPU adapter it uses: a `Gl` backend or a `Cpu`
device type means the GPU driver did not load and GPU phases will be slow.

## Phases

The phases call the same functions as the app. They run in this order:

| Phase          | What it times                                                         |
| -------------- | --------------------------------------------------------------------- |
| `decode`       | Decode and develop the full image (`load_base_image_from_bytes`)      |
| `rotate`       | Rotate the full image by 1.5° (`apply_rotation`)                      |
| `crop`         | Crop the full image (`apply_crop`)                                    |
| `warp`         | Perspective and distortion correction (`warp_image_geometry`)         |
| `downscale`    | Downscale to a 1920 px preview (`downscale_f32_image`)                |
| `gpu-upload`   | Full-size GPU render, including the texture upload                    |
| `gpu`          | Full-size GPU render of an already uploaded image, with style changes |
| `encode-jpeg`  | Encode the rendered image as JPEG, quality 90                         |
| `encode-tiff`  | Encode the rendered image as 16-bit TIFF                              |
| `thumbnail`    | Generate a library thumbnail (`generate_thumbnail_data`)              |
| `open`         | Open the image in the editor with empty caches (`load_image`)         |
| `first-frame`  | First editor preview after opening (`process_preview_job`)            |
| `style`        | Editor preview after a style change                                   |
| `drag`         | Live editor preview while dragging the exposure slider                |
| `geometry`     | Editor preview after a rotation change                                |
| `full-preview` | Full-resolution render with edits (`generate_preview_for_path`)       |
| `export`       | JPEG export without writing the file: decode, edit, render, encode    |

Every phase hashes its output. Editor previews are hashed by their decoded pixels, so two
JPEG encodings of the same pixels count as identical.

## Comparing builds

`compare-bench.mjs` compares two or more result files in order. For each phase it prints the
median time summed over the images, the change from the previous run, and marks a phase whose
output changed. It exits with status 1 if any output changed.

```bash
node bench/compare-bench.mjs before=bench/out/before.json after=bench/out/after.json
```

`bench-commits.sh` builds each given commit and runs the benchmark with each build, then
compares them. To check every merge on a branch's first-parent history, starting from its
base commit:

```bash
bench/bench-commits.sh $(git rev-list --first-parent --reverse <base>^..HEAD) -- photos/*.ARW --iters 3
```

All commits are built before any benchmark runs. The builds go to `bench/out/commits` (set
`BENCH_WORK` to change it) and are reused when the script runs again.

## Notes

- It reads the photos and their sidecars, and writes nothing next to them.
- Close other heavy programs, and keep a laptop plugged in: the results vary with CPU load,
  temperature and power mode.
- The command uses the headless export entry point, so it ends with "Headless export
  completed successfully."
