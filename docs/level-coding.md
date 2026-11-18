# Generator level coding (offline software model)

This document describes `src/generator/level.rs`, a pure Rust model based on the local ATS-1.5 analysis in `../docs/reverse-engineering/ats15-generator-level-output.md`. The module builds values and a software sequence plan only. It does not call USB/APIB, write to a DSP, sleep, or assert a physical output level, relay, attenuator, or mute effect.

## API and explicit inputs

- `GeneratorState` carries the logical master and A/B output flags, B-tracking flag, `OutputConfig`, unresolved/raw `ImpedanceState`, and current A/B range rows. `None` means unknown; it is not silently replaced.
- `LevelRequest` requires a channel, already-normalized Vrms value, explicit F15 bit, and (only when B tracking is active and B is enabled) A's normalized Vrms value. AP-Basic `dBu`, `dBV`, `dBm`, `W`, and `dBr` conversion is intentionally not included.
- `OutputConfig` determines the table input: `x=V` for balanced and `x=2V` for unbalanced/CMTST. The channel is explicit; no physical connector or channel-to-DSP-base mapping is inferred.
- `ImpedanceState::InternalCode` preserves a raw value without mapping it to 40/50/150/200/600 ohms.
- The documented input upper guard is applied as 16 Vrms for balanced or 8 Vrms for unbalanced/CMTST, multiplied by the recovered `1.00025` tolerance. Negative and non-finite values fail closed. No unverified minimum amplitude is invented.

## Range table and quantization

`RANGE_TABLE` copies all nine threshold/selector/scale rows from section 3.1 of the analysis report. Selection uses the first row in table order for which `threshold <= x`; equality belongs to that row. The six selector DWORDs remain raw data; only selector 0 is used to form word bit 14, as documented.

For selected row `i` and scale `K[i]`:

```text
q = min(8191, trunc(x * K[i] + 0.5))
word = (F15 << 15) | (selectors[i][0] << 14) | 0x2000 | ((~q) & 0x1fff)
low_byte = word & 0xff
high_byte = (word >> 8) & 0xff
```

`q` is calculated only for finite, nonnegative input. F15 is passed explicitly and preserved. A software-disabled channel (master off or selected channel off) returns row 8, `q=0`, and the conditional sentinel `F15<<15 | 0x7fff`; it is not evidence of zero volts at a connector.

### Conditional examples

With balanced configuration and F15=0:

| Explicit normalized input | Row | q | Word | Low, high |
|---|---:|---:|---:|---|
| 1.000 Vrms | 2 | 4096 | `0x2fff` | `0xff`, `0x2f` |
| 0.7746 Vrms (manual's 0 dBu reference) | 2 | 3172 | `0x339b` | `0x9b`, `0x33` |
| Channel logically disabled | 8 | 0 | `0x7fff` | `0xff`, `0x7f` |

The 0 dBu vector deliberately supplies `0.7746` Vrms directly. The AP-Basic unit conversion has not been reconstructed fully.

## Logical state and sequence planning

`plan_output_sequence` compares explicit before/target states and consumes an `EncodedLevel`. It returns actions, never executes them.

For an enabled channel with a known range change, the plan is:

1. `WriteTemporarySentinel` (only the low-13-bit all-ones intent is specified; the complete temporary control word is not guessed).
2. `WriteControlAndSelectors`, containing caller-supplied values for keys 0/4/5/8/9 and the target row's six raw selector values. No default control values are supplied; the grouped action does not assert ordering among those individual keys.
3. `WaitMilliseconds(5)` as an explicit plan step; unit tests do not sleep.
4. `WriteTargetWord`, carrying the final word and Low-then-High byte fields. The recovered byte offsets are basis+6 then basis+7; physical channel/basis mapping is unresolved.

If the row is unchanged, the plan contains only the final target-word action. If a required current/target row or control values are unknown, planning fails. Config, impedance, tracking, master, or channel-state transitions are rejected because their full sequence is not reconstructed here. For a logically disabled target, the planner returns a single `WriteOffSentinel` intent without a delay; this models the conditional software sentinel only.

`SequenceStep::WriteControlAndSelectors` is an abstract plan group, not an executable transport payload. The report's exact control values depend on runtime/configuration state and remain explicit inputs. No physical register or relay semantics are assigned.

## Verification and limits

Unit tests cover the two positive vectors, off sentinel, F15 and selector/marker-bit packing, all nine rows and threshold boundaries, invalid amplitudes, upper-limit saturation, unbalanced/CMTST normalization, tracked B input, ordered range-transition planning, unchanged-range planning, and missing/unsupported runtime state. They use no sleeps or hardware.

Still unresolved: AP-Basic unit conversion; exact per-key control writes and their internal ordering; F15 source/history; complete temporary word; impedance-code mapping; state transitions for output/config/impedance setters; channel-to-DSP-base mapping; APIO mode/status and device-object gates; and all real analog behavior. Do not treat the planner as permission or implementation for hardware writes.

Primary evidence: `../docs/reverse-engineering/ats15-generator-level-output.md`, especially §§3–6. That report itself cites the local manuals and ATS-1.5 binary addresses and distinguishes documented interface claims from static and conditional findings.
