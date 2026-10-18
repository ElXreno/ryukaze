# The advanced fan mode

Auto and silent run the fans on the EC's own curves, capped by the shift
mode (eco 43 %, comfort 60 %) and with the two fans running together: each
fan's register still holds what its own curve asks (0 % from a cool GPU),
so ryukaze reports the higher of the two for both. Advanced lifts the caps,
lets each fan go its own way, and runs them on your curves (or the EC's,
where you set none; see the [config](config.md)), in one of two ways:

- **Adaptive** (the default): ryukazed reads the temperatures every two
  seconds and smooths them, quick on the way up (10 s) and slow on the way
  down (60 s), so a burst of load doesn't send the fans chasing it; from
  90 °C a reading goes straight through. The speed comes off the curve
  straight between its dots, and the EC is held at it, with a step to full
  speed from 90 °C (CPU) or 85 °C (GPU) left in its table in case ryukazed
  isn't there to react. When ryukazed stops it hands the curves back to
  the EC. With `gpu_follows_cpu` (the default) the GPU fan runs at least as
  fast as the CPU's curve asks; otherwise it would stand still whenever the
  dGPU sleeps, as the EC reads its temperature as 0.
- **Stepping**: the curves go into the EC, which steps through them on its
  own temperatures.

In the window, the curve editor shows under the fan modes while advanced
is on (see [using ryukaze](usage.md#the-window)). The curves, the speeds and
the rpm live in the EC's RAM, which msi-ec doesn't cover: see the
[hardware notes](hardware.md#the-ecs-ram) for where, and on which firmware.
