# How music sync works

Music sync records what the computer plays (`pw-record`, from the monitor of the default
output) and analyses the last 85 ms of audio about 30 times a second over the cable, or up to
20 times a second over 2.4G. Each frame becomes one colour per key, sent as a live frame that
the keyboard shows but doesn't store.

## What each Follow choice listens to

| Follow | Analysis | On the keys |
|---|---|---|
| All | 18 bands from 40 Hz to 16 kHz, each with its own automatic gain | One bar per column. Bars rise at once and fall over about 0.1 s, slightly blended with their neighbours. |
| Bass | 18 bands from 30 to 300 Hz, with a longer 4096-sample window for finer steps | Bars over the whole board |
| Beat | Sudden rises in the lowest bands (spectral flux) against their recent average | A burst from the middle on every kick |
| Vocals | The singing voice (see below) | A glow centred on the column of the sung pitch, brighter when the voice is louder |

Ticked choices are combined: each column shows the brightest of them.

## Finding the voice

Most songs put the lead vocal in the centre of the stereo mix. In hip-hop, though, the kick,
the snare and the 808 are in the centre too, so "centre only" isn't enough. That is why
Vocals used to pulse with the beat. The voice is now found in four steps:

1. **Centre.** For each frequency, keep the part where left and right are alike over the last
   ~0.25 s: they must be correlated (coherence) and about equally loud. Wide synths, pads and
   reverb drop out.
2. **Harmonic, not percussive.** A sung note is steady over time and narrow in frequency; a
   drum hit is short and spread over many frequencies. Per frequency bin, compare
   - the median over the last 5 frames (steady), with
   - the median over the 17 neighbouring bins (broadband).

   Keep the bin in proportion to how steady it is: `steady² / (steady² + broadband²)`. This is
   the median-filter harmonic/percussive separation of Fitzgerald (2010), run on the last few
   frames so it works live.
3. **Voice range.** Only 300 Hz to 3.5 kHz counts. That excludes the 808 and its
   low harmonics, and the cymbals, hi-hats and breath above.
4. **Loudness and pitch.** The energy left is the voice level, with its own automatic gain.
   It rises in 30 ms and falls in 150 ms. The pitch comes from a harmonic search: for each
   candidate from 90 to 700 Hz, compare the energy at its harmonics with the energy half-way
   between them, which rules out octave mistakes. The glow glides to the pitch's column, with
   low notes left and high notes right.

### Measured

`bench/vocal_mix.py` makes a 24-second test mix with an answer key:

- a kick, snare and 808 in the centre, and hi-hats panned
- a wide pad
- a voice singing 2.5-second phrases with gaps, on changing notes

`bench/vocal_eval.py` scores it.

| | Before (v0.5.0) | Now |
|---|---|---|
| Brightness while singing ÷ in the gaps | 1.8 | 3.6 |
| Correlation of the lit column with the sung pitch | −0.07 | 0.76 |
| Frame-to-frame change while singing (lower is smoother) | 0.076 | 0.044 |

The same check ran on 2.5 minutes of real Filipino hip-hop and R&B, recorded from this computer
while it played and not included in the repo. The Vocals level's correlation with the beat fell
from +0.20 to +0.03, and with the bass from +0.49 to +0.35. Before, the Vocals lights were above
30 % brightness 94 % of the time, so there was almost no difference between singing and not.
Now that is 78 %, and the lights go dark between vocal lines.

With All, the average change per LED from one frame to the next halved on the same recording,
from 5.6 to 2.9 steps out of 255. The reaction to a sudden sound stayed the same, at 16 ms on
average from the audio arriving to the frame being sent (`bench/live.py`).

## What other visualizers do

- [scottlawsonbc/audio-reactive-led-strip](https://github.com/scottlawsonbc/audio-reactive-led-strip),
  a popular open-source LED strip visualizer, uses exponential filters that rise fast and decay
  slower (rise 0.99, decay 0.5 per frame on its mel spectrum), a slowly falling peak for
  automatic gain, and a Gaussian blur across LEDs. The author's write-up,
  [Audio Reactive LED Strips Are Diabolically Hard](https://scottlawsonbc.com/post/audio-led),
  explains why fixed thresholds fail and adaptive gain matters. The All bars here now use the
  same rise/decay idea and a light blur.
- [KeyboardVisualizer](https://github.com/FIX94/KeyboardVisualizer) (Razer Chroma, Corsair) has
  an *amplitude*, a *decay* (how much of the previous frame is kept) and a *filter constant*.
- [SteelSeries GG's Audio Visualizer](https://support.steelseries.com/hc/en-us/articles/48540958895501-How-to-Enable-Audio-Visualizer)
  offers colours, an amplitude scale and effects.
  [Corsair iCUE's Audio Visualizer](https://help.corsair.com/hc/en-us/articles/4407023238029-How-do-I-use-the-Audio-Visualizer-effect-in-iCUE)
  needs a Corsair headset as the output. Neither documents any smoothing or voice option.

None of these follow the voice separately. They show the spectrum, so a voice only shows as
part of the mids.

## Over 2.4G

The receiver takes the frame in 14-byte fragments, spaced 10 ms apart; 2 ms spacing stuttered.
A frame with every key lit needs about 8 fragments, so 2.4G manages 12–20 frames per second,
where the cable manages 30. Every distinct colour also costs bytes, so partly lit keys aren't
dimmed over 2.4G. For the smoothest music sync, use the cable.
