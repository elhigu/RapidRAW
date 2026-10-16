#!/usr/bin/env node
// Regression test for one-frame flicker at the end of UI animations on Linux.
//
// Runs RapidRAW in an off-screen Wayland compositor (headless sway), drives it with a virtual
// pointer (through wayvnc), records every output frame losslessly and fails when a frame differs
// from both of its neighbours while the neighbours match each other: a one-frame flash.
// See README.md for the requirements.
//
// Usage: node tests/linux-animation-flicker/check-flicker.mjs <RapidRAW binary> [--cycles n] [--keep]
// Exit status: 0 no flicker, 1 flicker found, 2 the test could not run or drive the app.
import { spawn, spawnSync } from 'node:child_process';
import { chmodSync, createWriteStream, mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import net from 'node:net';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

// The off-screen output and the positions of the controls in RapidRAW's default layout at this size.
const SCREEN = { width: 1920, height: 1080 };
const SLIDER_LABEL = { x: 1688, y: 344 }; // right end of the "Contrast" label in the Basic section
const ADJUSTMENTS_BUTTON = { x: 1887, y: 81 };
const MASKS_BUTTON = { x: 1887, y: 160 };
const AWAY = { x: 1300, y: 900 }; // empty filmstrip area

const SCENARIOS = [
  {
    name: 'slider-label',
    description: 'hover a slider label, which crossfades to "Reset", and leave it again',
    region: { x: 1590, y: 320, width: 300, height: 60 },
    steps: [
      ['move', SLIDER_LABEL],
      ['wait', 700],
      ['move', AWAY],
      ['wait', 700],
    ],
  },
  {
    name: 'panel-switch',
    description: 'switch between the Adjustments and Masks panels',
    region: { x: 1600, y: 60, width: 240, height: 540 },
    steps: [
      ['click', MASKS_BUTTON],
      ['wait', 1000],
      ['click', ADJUSTMENTS_BUTTON],
      ['wait', 1000],
    ],
  },
];

// A frame counts as changed when its mean absolute difference from another frame exceeds this
// (on a 0-255 scale); compression is lossless, so identical frames differ by exactly 0.
const CHANGE = 0.05;

function parseArgs(argv) {
  const options = { binary: null, cycles: 5, keep: false };
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === '--cycles') options.cycles = Number(argv[++i]);
    else if (argv[i] === '--keep') options.keep = true;
    else if (!options.binary) options.binary = resolve(argv[i]);
    else throw new Error(`Unknown argument: ${argv[i]}`);
  }
  if (!options.binary || !(options.cycles > 0)) {
    throw new Error('Usage: check-flicker.mjs <RapidRAW binary> [--cycles n] [--keep]');
  }
  return options;
}

const sleep = (ms) => new Promise((done) => setTimeout(done, ms));

async function waitFor(what, timeoutMs, check) {
  const end = Date.now() + timeoutMs;
  while (Date.now() < end) {
    const result = await check();
    if (result) return result;
    await sleep(250);
  }
  throw new Error(`Timed out waiting for ${what}`);
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { maxBuffer: 1 << 30, ...options });
  if (result.error) throw new Error(`Cannot run ${command}: ${result.error.message}`);
  if (result.status !== 0) {
    throw new Error(`${command} failed (${result.status}): ${result.stderr?.toString().trim()}`);
  }
  return result.stdout;
}

// Minimal RFB (VNC) client: enough to move the virtual pointer and click.
class Pointer {
  static connect(port) {
    return new Promise((done, fail) => {
      const socket = net.connect(port, '127.0.0.1');
      let buffer = Buffer.alloc(0);
      let stage = 'version';
      socket.on('error', fail);
      socket.on('data', (data) => {
        buffer = Buffer.concat([buffer, data]);
        if (stage === 'version' && buffer.length >= 12) {
          buffer = buffer.subarray(12);
          socket.write('RFB 003.008\n');
          stage = 'security';
        }
        if (stage === 'security' && buffer.length >= 1 && buffer.length >= 1 + buffer[0]) {
          const types = [...buffer.subarray(1, 1 + buffer[0])];
          if (!types.includes(1)) fail(new Error(`wayvnc offers no unauthenticated access: ${types}`));
          buffer = buffer.subarray(1 + buffer[0]);
          socket.write(Buffer.from([1]));
          stage = 'result';
        }
        if (stage === 'result' && buffer.length >= 4) {
          if (buffer.readUInt32BE(0) !== 0) fail(new Error('VNC security handshake failed'));
          buffer = buffer.subarray(4);
          socket.write(Buffer.from([1])); // ClientInit: shared session
          stage = 'init';
        }
        if (stage === 'init' && buffer.length >= 24 && buffer.length >= 24 + buffer.readUInt32BE(20)) {
          stage = 'ready';
          done(new Pointer(socket));
        }
      });
    });
  }

  constructor(socket) {
    this.socket = socket;
    this.buttons = 0;
  }

  send(x, y) {
    const message = Buffer.alloc(6);
    message.writeUInt8(5, 0); // PointerEvent
    message.writeUInt8(this.buttons, 1);
    message.writeUInt16BE(x, 2);
    message.writeUInt16BE(y, 4);
    this.socket.write(message);
  }

  async move({ x, y }) {
    this.send(x, y);
  }

  async click({ x, y }) {
    this.send(x, y);
    await sleep(50);
    this.buttons = 1;
    this.send(x, y);
    await sleep(50);
    this.buttons = 0;
    this.send(x, y);
  }

  close() {
    this.socket.destroy();
  }
}

// Mean absolute difference per channel between two RGB frames.
function difference(a, b) {
  let sum = 0;
  for (let i = 0; i < a.length; i++) sum += Math.abs(a[i] - b[i]);
  return sum / a.length;
}

function loadFrames(video, region) {
  const raw = run('ffmpeg', [
    '-v',
    'error',
    '-i',
    video,
    '-fps_mode',
    'passthrough',
    '-f',
    'rawvideo',
    '-pix_fmt',
    'rgb24',
    '-',
  ]);
  const size = region.width * region.height * 3;
  const frames = [];
  for (let offset = 0; offset + size <= raw.length; offset += size) frames.push(raw.subarray(offset, offset + size));
  return frames;
}

function analyze(frames) {
  const changed = frames.map((frame, i) => i > 0 && difference(frame, frames[i - 1]) > CHANGE);
  // Animations show up as runs of consecutive changed frames.
  let animations = 0;
  for (let i = 1; i < changed.length; i++) if (changed[i] && !changed[i - 1]) animations++;
  const flashes = [];
  for (let i = 1; i < frames.length - 1; i++) {
    const previous = difference(frames[i], frames[i - 1]);
    const next = difference(frames[i], frames[i + 1]);
    const around = difference(frames[i - 1], frames[i + 1]);
    if (Math.min(previous, next) > CHANGE && around < Math.min(previous, next) / 3) flashes.push(i);
  }
  return { animations, flashes };
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  const work = mkdtempSync(join(tmpdir(), 'rapidraw-flicker-'));
  const runtime = join(work, 'runtime');
  mkdirSync(runtime);
  chmodSync(runtime, 0o700);
  for (const dir of ['config', 'data', 'cache']) mkdirSync(join(work, dir));
  const log = (name) => createWriteStream(join(work, `${name}.log`));
  const children = [];
  const start = (name, command, args, env) => {
    const child = spawn(command, args, { env, stdio: ['ignore', 'pipe', 'pipe'] });
    const out = log(name);
    child.stdout.pipe(out);
    child.stderr.pipe(out);
    child.on('error', (error) => out.write(`${error}\n`));
    children.push(child);
    return child;
  };
  let failed = true;

  try {
    const image = join(work, 'test.jpg');
    run('ffmpeg', ['-v', 'error', '-f', 'lavfi', '-i', 'testsrc2=size=1600x1067', '-frames:v', '1', image]);
    const swayConfig = join(work, 'sway.conf');
    writeFileSync(
      swayConfig,
      `output HEADLESS-1 resolution ${SCREEN.width}x${SCREEN.height}@60Hz\ndefault_border none\n`,
    );

    const baseEnv = { ...process.env, XDG_RUNTIME_DIR: runtime };
    delete baseEnv.WAYLAND_DISPLAY;
    delete baseEnv.DISPLAY;
    delete baseEnv.SWAYSOCK;
    start('sway', 'sway', ['-c', swayConfig], { ...baseEnv, WLR_BACKENDS: 'headless', WLR_LIBINPUT_NO_DEVICES: '1' });
    const display = await waitFor('sway', 20000, () => readdirSync(runtime).find((f) => /^wayland-\d+$/.test(f)));
    const env = { ...baseEnv, WAYLAND_DISPLAY: display };

    const vncPort = 5900 + 100 + (process.pid % 800);
    start('wayvnc', 'wayvnc', ['-o', 'HEADLESS-1', '127.0.0.1', String(vncPort)], env);
    const pointer = await waitFor('wayvnc', 20000, () => Pointer.connect(vncPort).catch(() => null));

    start('rapidraw', options.binary, [image], {
      ...env,
      GDK_BACKEND: 'wayland',
      XDG_CONFIG_HOME: join(work, 'config'),
      XDG_DATA_HOME: join(work, 'data'),
      XDG_CACHE_HOME: join(work, 'cache'),
    });

    // Ready when the right panel has content and has stopped changing for a second.
    const grab = (r) => run('grim', ['-g', `${r.x},${r.y} ${r.width}x${r.height}`, '-t', 'ppm', '-'], { env });
    const panel = SCENARIOS[1].region;
    let last = null;
    let stableSince = 0;
    await waitFor('the editor to open', 90000, () => {
      const shot = grab(panel);
      const pixels = shot.subarray(shot.length - panel.width * panel.height * 3);
      const varied = pixels.some((v) => Math.abs(v - pixels[0]) > 40);
      if (!varied || !last || !last.equals(pixels)) {
        last = pixels;
        stableSince = Date.now();
        return false;
      }
      return Date.now() - stableSince > 1500;
    });
    await sleep(1000);

    let flickered = false;
    let undriven = false;
    for (const scenario of SCENARIOS) {
      const { region } = scenario;
      const video = join(work, `${scenario.name}.mkv`);
      const recorder = start(
        `record-${scenario.name}`,
        'wf-recorder',
        [
          '-y',
          '-D',
          '-o',
          'HEADLESS-1',
          '-g',
          `${region.x},${region.y} ${region.width}x${region.height}`,
          '-c',
          'libx264rgb',
          '-p',
          'crf=0',
          '-p',
          'preset=ultrafast',
          '-f',
          video,
        ],
        env,
      );
      await sleep(1000);
      for (let cycle = 0; cycle < options.cycles; cycle++) {
        for (const [action, value] of scenario.steps) {
          if (action === 'wait') await sleep(value);
          else await pointer[action](value);
        }
      }
      await sleep(500);
      recorder.kill('SIGINT');
      await waitFor('wf-recorder to finish', 15000, () => recorder.exitCode !== null || recorder.signalCode !== null);

      const frames = loadFrames(video, region);
      const { animations, flashes } = analyze(frames);
      const expected = 2 * options.cycles;
      const verdict = animations < expected ? 'NOT DRIVEN' : flashes.length > 0 ? 'FLICKER' : 'ok';
      console.log(
        `${verdict.padEnd(10)} ${scenario.name}: ${scenario.description}\n` +
          `           ${frames.length} frames, ${animations} animations (expected at least ${expected}), ` +
          `${flashes.length} one-frame flashes${flashes.length ? ` at frames ${flashes.join(', ')}` : ''}`,
      );
      if (animations < expected) undriven = true;
      if (flashes.length > 0) flickered = true;
    }
    pointer.close();

    failed = flickered || undriven;
    if (undriven) {
      console.log('\nThe app did not react to every step: the layout may have changed, or the app did not start.');
      process.exitCode = 2;
    } else if (flickered) {
      console.log('\nFlicker found: a frame at the end of an animation shows an earlier state.');
      process.exitCode = 1;
    } else {
      console.log('\nNo flicker found.');
    }
  } catch (error) {
    console.error(`Error: ${error.message}`);
    process.exitCode = 2;
  } finally {
    for (const child of children.reverse()) child.kill('SIGTERM');
    if (failed || options.keep) console.log(`Recordings and logs: ${work}`);
    else rmSync(work, { recursive: true, force: true });
  }
}

// Run in a private D-Bus session, so the app doesn't hand over to a RapidRAW that is already
// running on the desktop (single-instance plugin) and nothing leaks into the user's session. The
// bus starts no services, such as portals, on demand.
if (process.env.RAPIDRAW_FLICKER_TEST_PRIVATE_BUS) {
  await main();
} else {
  const dir = mkdtempSync(join(tmpdir(), 'rapidraw-flicker-bus-'));
  const config = join(dir, 'session.conf');
  writeFileSync(
    config,
    `<busconfig>
  <type>session</type>
  <listen>unix:dir=${dir}</listen>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
`,
  );
  const result = spawnSync(
    'dbus-run-session',
    [`--config-file=${config}`, '--', process.execPath, ...process.argv.slice(1)],
    {
      stdio: 'inherit',
      env: { ...process.env, RAPIDRAW_FLICKER_TEST_PRIVATE_BUS: '1' },
    },
  );
  rmSync(dir, { recursive: true, force: true });
  if (result.error) {
    console.error(`Cannot run dbus-run-session: ${result.error.message}`);
    process.exit(2);
  }
  process.exit(result.status ?? 2);
}
