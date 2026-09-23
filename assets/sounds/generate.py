# Synthesizes ZapFast's notification sounds from scratch (no samples). See README.md.
import numpy as np, wave, sys
RATE = 48000

def note(freq, dur=0.55, tau=0.16, bright=1.0):
    t = np.arange(int(RATE * dur)) / RATE
    # Mallet-like partials: fundamental, octave, and a soft inharmonic "bell" tone.
    partials = [(1.0, 1.0, tau), (2.0, 0.22 * bright, tau * 0.6), (3.01, 0.06 * bright, tau * 0.4), (4.2, 0.025 * bright, tau * 0.25)]
    out = np.zeros_like(t)
    for ratio, amp, decay in partials:
        out += amp * np.sin(2 * np.pi * freq * ratio * t) * np.exp(-t / decay)
    attack = np.minimum(1.0, t / 0.004)  # 4 ms attack avoids clicks
    return out * attack

def mix(events, length):
    buf = np.zeros(int(RATE * length))
    for start, sig, gain in events:
        i = int(RATE * start)
        n = min(len(sig), len(buf) - i)
        buf[i:i + n] += gain * sig[:n]
    return buf

def room(sig):
    # A touch of early reflections for warmth.
    out = sig.copy()
    for delay, gain in [(0.023, 0.18), (0.041, 0.12), (0.067, 0.07)]:
        d = int(RATE * delay)
        out[d:] += gain * sig[:-d]
    return out

def finish(sig, peak_db=-7.0):
    fade = int(RATE * 0.05)
    sig[-fade:] *= np.linspace(1, 0, fade)
    sig = sig / np.max(np.abs(sig)) * 10 ** (peak_db / 20)
    return sig

def write(path, sig):
    data = (np.clip(sig, -1, 1) * 32767).astype('<i2')
    with wave.open(path, 'wb') as w:
        w.setnchannels(1); w.setsampwidth(2); w.setframerate(RATE); w.writeframes(data.tobytes())

# One-to-one message: two notes rising a fourth (C6 -> F6).
message = mix([(0.0, note(1046.5), 0.8), (0.085, note(1396.9), 1.0)], 0.7)
# Group message: a quicker three-note arpeggio (G5 -> C6 -> E6), softer top.
group = mix([(0.0, note(784.0, bright=0.8), 0.75), (0.07, note(1046.5, bright=0.8), 0.85), (0.14, note(1318.5, tau=0.2), 0.9)], 0.8)
write(sys.argv[1] + '/chime.wav', finish(room(message)))
write(sys.argv[1] + '/ripple.wav', finish(room(group)))
