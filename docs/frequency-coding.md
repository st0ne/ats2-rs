# Offline frequency coding

This document describes `generator::frequency`, a pure Rust model of the ATS-1.5 S2DLL path in `../../docs/reverse-engineering/ats15-primary-output-frequency.md`. It computes values only: no USB, APIB, device state, GUI, or hardware calls are present. Internal identifiers `0x30` and `0x31` are **not established hardware-register addresses**.

## Reconstructed input state

`FrequencyRuntimeState` requires the caller to provide:

- the active `ActiveRateSource` and its resolved nonzero sample rate; unresolved rates return an error rather than selecting a default;
- the waveform code `[0x1005e6a8]` and mode status `[0x1005e26c]`;
- explicit `system_check` and `rate_source_initialization` gates;
- optional offset `C`; `None` remains unknown and cannot be converted to zero.

The static RateSource resolver maps selector 0 to 48,000 Hz, 1/2 to 55,000 Hz, and 3 to 65,536 Hz. Selector 6 maps to 131,072 Hz only for nonzero mode status with the `[0x1005e270]` gate clear. Other selectors and unavailable/unknown selector-6 conditions return errors. These are code-path mappings, not an assertion about which source is active on a particular instrument.

## Mode-dependent equations

The mode precheck uses `minimum = A × 0.999` and `maximum = Hcheck × 1.00025`. The quantizer separately clamps to `[L,H]`, where `L=0` and `H=Fs/(2g)`. In resolved branches, `s=A×g/Fs`, except the mode-5 nonzero-status branch, which calls the alternate helper with `s=2/Fs` after retaining the 4,500-Hz lower precheck.

| Waveform code | Lower reference A | Hcheck | g | Quantizer H | s |
|---|---:|---|---:|---|---|
| 4, 12, 13 | 2,000 | `0.47×Fs` | 1 | `Fs/2` | `2,000/Fs` |
| 5, status 0 | 4,500 | `0.47×Fs` | 1 | `Fs/2` | `4,500/Fs` |
| 5, status nonzero | 4,500 for precheck | `0.47×Fs` | 1 | `Fs/2` | `2/Fs` (alternate helper) |
| 11 | 2 | `0.235×Fs` | 1 | `Fs/2` | `2/Fs` |
| 18 | 2 | `min(0.47×Fs, 40,000)` | 2 | `Fs/4` | `4/Fs` |
| Other codes (including 0) | 2 | `0.47×Fs` | 1 | `Fs/2` | `2/Fs` |

Thus `s=1/Fs` is not a reconstructed branch. Modes 4/12/13 and 5 reject 1 kHz at the lower precheck; mode 11, mode 18, and the standard mode accept it subject to the separate system-check gate.

For accepted finite positive input `v`, the implementation evaluates the equivalent documented path:

```text
x = clamp(v, 0, H) × s
q = trunc(x × 2^23 + 0.49999)
q ∈ [0, 2^23−1]
```

Positive conversion truncates toward zero, not round-to-nearest. The positive result is capped at `8,388,607`. The full setter rejects values outside the reconstructed mode guard; this means the internal helper's negative/special fallback is not exposed as a successful Rust result. NaN and infinities are rejected explicitly (fail-closed policy).

The DLL uses x87 instructions and a runtime-selected conversion helper (`[0x100645c8]`); the report supports truncation toward zero for positive normal values but does not establish every intermediate's bit-exact behavior at a one-ULP boundary. This implementation uses explicit Rust `f64` operations. The 1-kHz vector is not near an integer boundary; boundary tests verify the documented model, not physical hardware behavior.

The static mode guards correspond to the S2DLL's lower/upper status branches (reported as status 5/6); Rust represents them as typed errors. Device-dataset limits checked by `fcn.100193f5` remain an explicit caller-provided gate, not invented constants.

## Conditional normal-Sine vector

The analyzed Rabbit Sine-family setter calls `RateSourceAoutDsp(1)` before the waveform setter; that static branch selects 55,000 Hz. The first Sine-family table entry (external code 32 to internal mode 0) is the **conditional Normal-Sine candidate**; the resource-label-to-index binding is not fully proved. After that mode setter completes successfully, the subsequent `Freq=1000 Hz` property operation has the following conditional vector, assuming no later RateSource change and known `C` history:

```text
Fs = 55,000 Hz; mode = 0; A = 2; g = 1
s = 2/55,000; L = 0; H = 27,500
q = trunc(1000 × (2/55,000) × 8,388,608 + 0.49999)
  = 305,040 = 0x0004A790
C = 305,040 (successful preceding Normal-Sine IMD update)
0x30 → q−C = 0
0x31 → q+C = 610,080 = 0x00094F20
```

`calculate_dsp_write_data` returns these signed 32-bit data values and the internal low identifiers `0x30`/`0x31`. Checked add/sub rejects signed 32-bit overflow. The function does not serialize bytes or initiate either of the two `DspObj::WriteToDspAddr` operations.

## Evidence and limits

**Statically supported:** mode branches and constants listed above; the selected-rate mappings; positive quantization and saturation; the `q−C`/`q+C` formulas; the internal address identifiers; and conditional APIB object/token/payload paths described in the source report.

**Still runtime-conditional:** whether the Sine setter ran before the property call, whether another RateSource setter changed Fs afterward, the current `C` after prior successful/failed IMD writes, system-dataset check outcome, DspObj presence/status gates, and the APIO backend/status/retry result. The module models only arithmetic prerequisites; APIB and transport behavior intentionally remain elsewhere and are not implemented here.

**Requires authorized hardware/integration testing:** any claim that these software payloads produce a particular analog frequency, that `0x30`/`0x31` have a particular device-side meaning, or that the transport succeeds on a given OS/driver/adapter. The unit tests are offline and do not establish those claims.
