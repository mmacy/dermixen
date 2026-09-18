# Audio clip fixtures

Audio files for tests that cannot use audio generated in memory: decoding files written by real encoders, reading tags, and finding beats in music.

Every file in this directory has an entry in the table below giving where it came from, why it may be committed, and what it is for. A file without an entry is not accepted. Real clips of music are kept to ten to twenty seconds.

| File | Source | Why it may be committed | Used by |
| --- | --- | --- | --- |
| `sine-440-44k.wav` | `tools/make_audio_fixtures.py`: two seconds of a 440 Hz sine, left at half scale and right at quarter scale, 16-bit stereo at 44.1 kHz | Synthetic | Decode tests, exact contents |
| `sine-440-48k.wav` | The same tone at 48 kHz | Synthetic | Decode tests, resampling |
| `sine-220-mono-44k.wav` | One second of a 220 Hz sine at half scale, 16-bit mono at 44.1 kHz | Synthetic | Decode tests, mono handling |
| `sine-440-44k.mp3` | `sine-440-44k.wav` encoded by LAME at 192 kbps constant bit rate | Synthetic | Decode tests, MP3 |
| `sine-440-44k.flac` | `sine-440-44k.wav` encoded by the reference FLAC encoder | Synthetic | Decode tests, FLAC |
| `sine-440-44k.m4a` | `sine-440-44k.wav` encoded by ffmpeg's AAC encoder at 128 kbps in an MP4 container | Synthetic | Decode tests, MP4 |
| `tagged.mp3` | `sine-440-44k.wav` encoded by LAME at 128 kbps with ID3v2 tags: artist `Slinky Wizard`, title `Lunar Juice (Hallucinogen Moon Strudel Remix)`, year 1996 | Synthetic | Tag reading |
| `tagged-blank.mp3` | The same, with the artist tag set and the title tag holding only spaces | Synthetic | Tag reading, blank values |
| `tagged.flac` | `sine-440-44k.wav` encoded by the reference FLAC encoder with the same artist, title, and date as Vorbis comments | Synthetic | Tag reading, FLAC |
| `tagged.m4a` | `sine-440-44k.wav` encoded by ffmpeg's AAC encoder with the same artist, title, and date as iTunes-style tags | Synthetic | Tag reading, MP4 |
