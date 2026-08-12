# The README's GIF, as a recipe

```sh
bash tools/demo/build-store.sh
python3 tools/demo/record.py
agg --theme 12121a,d8d8e0,12121a,f07178,6ad4a0,ffcb6b,82aaff,c792ea,89ddff,d8d8e0,5c6370,f07178,6ad4a0,ffcb6b,82aaff,c792ea,89ddff,ffffff \
    --font-size 16 --line-height 1.4 \
    tools/demo/loop.cast assets/leteo-loop.gif
```

`leteo` has to be on `PATH`, and `jq` and [`agg`](https://github.com/asciinema/agg)
have to be installed. `agg` ships a single static binary per platform.

The theme is Leteo's own, and it is a positional list rather than a name:
background, foreground, then the eight colours and their bright variants.

## What each step does

`build-store.sh` writes three memories about a payments service into a store
under `tools/demo/store/`, which is gitignored and rebuilt in a second. It is
never the real one — every command carries `--database`.

`record.py` writes an asciicast: the typing animated character by character,
and the output of each command as the binary actually printed it. It runs the
commands; it does not transcribe them. A command that prints nothing stops the
recording rather than producing a GIF of an empty screen.

`agg` rasterises that cast to a GIF with no browser involved.

## Why not vhs

vhs recorded the earlier GIFs and stopped working on the machine this project
is developed on. It prints its `Set` directives and then hangs before executing
a line — on a two-line tape as much as on a long one, with and without
`Set Shell bash`, from a real terminal and from an automated one. It drives a
headless browser to rasterise the terminal, and that is the part that broke.

Nothing in the resulting image says which machine it came from: the prompt is
drawn by the script, the commands are the same, and the binary is the same
product built for another platform. So the recording moved to whichever machine
can do it, and the pipeline lost its heaviest dependency on the way.
