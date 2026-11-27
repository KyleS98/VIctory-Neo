# VIctory-Neo

**The Xbox Series build of the title, running on a Windows PC from your own
files: about 31 frames a second, and about 53 on screen with frame generation.**

VIctory-Neo is a build of [VIctory](https://github.com/Marvisak/VIctory), the
compatibility layer that runs an Xbox Series title natively on Windows. The
title's own x86-64 code runs on your processor; the layer provides the
console's system calls and translates its graphics to Direct3D 12.

> **No game files are here, and none ever will be.** You need your own legally
> obtained dump of the title's Xbox Series build v1.0.2.1. This project is not
> affiliated with or endorsed by the title's developer or publisher, nor by
> NVIDIA. See [docs/asset-policy.md](docs/asset-policy.md).

## What it does today

Measured on one PC: **GeForce RTX 3090 Ti 24 GB** (driver 616.92), Ryzen 9
9950X3D, 64 GB RAM, Windows 11. Frame rates are the layer's own count (F3
shows it), medians over the 5-second readings of a run, hands off.

| Scene | Profile | Frames a second |
|---|---|---|
| Prologue, the cell (4,300 to 4,700 draws a frame) | Series S, 1920x1080 out | **31** (readings 27 to 33) |
| the same | Series S, DLSS Quality and AMD FSR 3 frame generation | **27 rendered, 53 shown** (F3: `FPS: 26.4  FG: 52.8`) |
| Opening cutscene | Series S | about 28 on average, 20 to 37 by shot |
| Prologue, the cell | Series X, scene 1920x1080, 2560x1440 out | 26 |
| Prologue, the cell | Series X, native 2560x1440 | 23 |

- The boot screens, the account picker, the loading art and the opening
  cutscene play in order, and the prologue's first room stands.
- Characters, hair, clothes and their prints, textures, sky and sun are drawn.
- No crash and no lost graphics device in the runs these numbers come from
  (a dozen, 5 to 10 minutes each, standing in the cell). Longer sessions,
  play with a pad or keyboard and mouse, and anything past the prologue are
  **not measured here yet**; the projects this is built on have played to the
  first boat mission (see [Built on](#built-on)).

On the same PC, fxceless/victory-fork's `main` of the same day runs the same
cell at 17 to 19 frames a second.

### What is not right yet

- **No ray tracing.** The title traces its indirect light, reflections and
  some shadows itself, and no layer runs that fast enough yet. An estimate
  stands in for indirect light and reflections (foxhound's work): rooms are
  lit, reflections are blurry. `VICTORY_BVH_BUILDER=1` runs the title's own
  rays: a few frames a second and a broken picture.
- **Haze through windows.** Bright views through glass (the cell's window)
  are smeared; cause not found yet.
- **Hitches** when the title streams a new scene in, and during the first
  launches while shaders are translated and pipelines made (see below).
- **Above 30 frames a second the game runs a little fast.** The title steps
  its world at least 1/30 s a frame. `--fps 30` holds it at its own rate.
  (Frames made by frame generation do not count: 27 rendered is 27.)
- **Frame generation adds latency** (about one rendered frame), as it does
  in any game, and its in-between frames can smear fast motion.
- **Memory.** On this PC it takes up to 43 GB of RAM, with a 24 GB card that
  it fills with textures. Close what you can. foxhound measures 20 GB in use
  (28 GB committed) on a 32 GB PC with a 12 GB card.

## Quick start

You need: Windows 10 or 11, a Direct3D 12 card with mesh shaders and shader
model 6.6 (measured on NVIDIA RTX; AMD runs, with less attention), 32 GB of
RAM or more, an SSD, [Rust](https://rustup.rs) and the Visual Studio Build
Tools with "Desktop development with C++".

```powershell
git clone https://github.com/KyleS98/VIctory-Neo
cd VIctory-Neo
cargo build --release --locked -p victory
target\release\victory.exe play "D:\MyDump\game_scarlett_master.exe" --series-s
```

Put the path of **your** `game_scarlett_master.exe` in the last line. That is
all: the settings these numbers were measured with are built in.

**More on screen on an RTX 20 or 30** (AMD FSR 3 frame generation, with DLSS
doing the upscaling; 53 shown here):

```powershell
target\release\victory.exe get-dlss     # once: NVIDIA's DLSS files, from NVIDIA's GitHub
target\release\victory.exe get-fsr      # once: AMD's FSR 3 files, from AMD's GitHub
target\release\victory.exe play "D:\MyDump\game_scarlett_master.exe" --series-s --dlss quality --framegen 2 --fg-type fsr
```

On an RTX 40 or 50, `--framegen 2` (or more, on an RTX 50) uses DLSS frame
generation instead; that was not measured here.

- **The first launches are slow, and that is expected.** The first start
  translates the title's shaders (minutes) and the driver compiles each
  pipeline the first time it is drawn. A launch may even end in the title's
  own watchdog while that goes on; start it again. Each launch writes down
  what it needed, so it gets smoother over the first two or three.
- **The first start asks once whether to download NVIDIA's DLSS files**
  (RTX cards). Either way works: in the cell DLSS Quality measured 31.4 and
  Performance 31.7 against 30.9 without it.
  The card's time there goes to the number of commands, not to pixels.
- **Known crash, with a way round it:** with AMD's FSR 3 files fetched and
  `--dlss off`, the start prepares FSR 3 and the program ends during the boot
  screens ([report](https://github.com/trustno1-coder/VIctory-NVIDIA/issues/459)). Add `--no-framegen-ready`.
- **F3** shows the stats panel, **F1** the settings menu (console profile,
  picture size, frame rate cap, the light estimate and the main switches).
- Nothing is written into your game folder: caches, saves, settings and logs
  are under `%LOCALAPPDATA%\victory`.

Other ways to run it:

| | |
|---|---|
| `victory.exe play <exe>` | Series X profile at 2560x1440: sharper, slower (23 here) |
| `victory.exe play <exe> --res 1080to1440` | Series X, scene at 1920x1080 (26 here) |
| `victory.exe play <exe> --series-s --fps 30` | the title's own frame rate, game speed exact |
| `victory.exe play <exe> --series-s --set VICTORY_X=1` | add or override a switch |
| `scripts\play-foxhound.ps1 -Exe <exe> -SeriesS` | the same from PowerShell, with a log file |

**DLSS**, **DLSS frame generation** (RTX 40 and 50) and **AMD FSR 3 frame
generation** (any card) come with the tree this is built on. Their files are
NVIDIA's and AMD's, under their licences, fetched from their own GitHub by
`victory.exe get-dlss` and `get-fsr` and never part of this repository;
[docs/dlss.md](docs/dlss.md) has the details.

## What Neo adds

Two changes on top of the tree it is built on, each measured in the game on
the PC above and sent to that project as a pull request:

1. **The Series S profile keeps the title's own upscaler** where no DLSS is
   set up. The title picks FSR 4 on a Series S; translated, that is the
   dearer of the two. 28.5 to 30.9 frames a second in the cell, 88 to 38 MB
   of title memory copied a frame. (With DLSS or frame generation set up the
   title's FidelityFX path is left as it is: they need it.)
2. **A pipeline cache left by other launches is dropped once.** Here a cache
   filled by launches of other configurations drew a shirt without its
   print, and a fresh one drew it right. Neo drops a cache it has not
   written its list of static samplers beside, once; every launch then
   makes those samplers in the same order. That order is what goes wrong
   on fxceless's tree ([measured](https://github.com/fxceless/victory-fork/issues/1459)),
   but on this tree pipelines with static samplers never come from the
   cache, so **the cause of the missing print here is not known yet**: the
   fresh cache is what fixed it, not the order. (Pull request in draft
   until that is tested.)
In progress, not in the build: **the console's per-lane descriptor loops run
once instead of once per distinct value** (a [pull request](https://github.com/trustno1-coder/VIctory-NVIDIA/pull/458),
now in draft). The translator proves only some of these loops and runs the
rest as loops that cost four group barriers a pass on a card whose waves are
half as wide as the console's. A check of the loop's shape proved 32,106
more of them, and the cell went from 31 to 32 to 33 frames a second (62 shown
with frame generation). But in 3 runs of 5 a wall in the cell then showed a
dark patch with tile-aligned edges, which it never shows without the change,
so it was taken out again until the cause is found.

Also in the tree, off by default: `VICTORY_GUARDS_SPIN=<microseconds>`, a
shorter wait on the write tracker's lock. A profile had the thread that
prepares the card's work asleep behind that lock 2 ms of every 32 ms frame.
With the switch at 40 that thread's time fell by 2 ms in one pair of runs
and by nothing in another, and the frame rate did not move in either: the
graphics card is as close to its limit as that thread is. It is there for
whoever measures on a faster card.

Found with the tree's own tools: its frame line and timing run, its sampling
profiler, and A/B switches that flip while the game runs.

## Built on

Neo is the newest link of a chain, and nearly all of the code is other
people's:

- **[Marvisak/VIctory](https://github.com/Marvisak/VIctory)**: the loader, the
  runtime, the Direct3D 12 back end and the shader recompiler.
- **[fxceless/victory-fork](https://github.com/fxceless/victory-fork)** and the
  people whose pull requests it carries: where most of the layer grew up.
- **[readmet3xt-lab/foxhound](https://github.com/readmet3xt-lab/foxhound)**:
  the base that first held 30 frames a second: list calls translated on
  eight threads, work done on the card instead of waited for, the light
  estimate, fewer stalls.
- **[trustno1-coder/VIctory-NVIDIA](https://github.com/trustno1-coder/VIctory-NVIDIA)**:
  the tree this one is taken from (its `main` of 2026-11-27): the launcher
  built into the program, the F1 menu, the stats panel, DLSS and frame
  generation, and the pull requests of its contributors.
- And everyone those name: avatarmanz, DexNom, Belobug, Vice-Tay and more.
  Their READMEs are kept here: [VIctory on NVIDIA](docs/README-nvidia.md),
  [foxhound](docs/README-foxhound.md), [fxceless](docs/README-fxceless.md),
  with what each found in [docs/findings.md](docs/findings.md).

Before 2026-11-27 `main` held an earlier snapshot of fxceless's line. It is
still in `main`'s history, and our frame-rate work on that line (20 to 22
frames a second in the same cell, with a print missing from the picture) is
on the `integration/*` branches.

## Reporting a problem

Open an issue with your card, driver, processor, RAM, the command you ran
and what you saw, in words. The log of a run is under
`%LOCALAPPDATA%\victory\logs`. **Never attach anything from the game**: no
files, dumps, shader dumps or screenshots.

## License

GPL-3.0-or-later, as VIctory. See [LICENSE](LICENSE); the one file that is
not covered is named in [docs/asset-policy.md](docs/asset-policy.md).

Much of this code, in every link of the chain, was written with AI
assistance (here: Claude Code).
