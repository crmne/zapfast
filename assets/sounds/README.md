# Notification sounds

`chime.ogg` (one-to-one chats) and `ripple.ogg` (groups) are original sounds
synthesized from sine partials by `generate.py`, with no recorded samples.
They are part of ZapFast and share its license.

Regenerate them with:

```sh
python3 generate.py .
for name in chime ripple; do
  ffmpeg -y -i $name.wav -c:a libvorbis -q:a 5 $name.ogg && rm $name.wav
done
```
