//! Reading the frame headers of an MP3 file, so a test can check what bit
//! rate a file was written at without decoding it.

/// The bit rates an MPEG-1 layer III frame header can name, in kilobits
/// per second, by the header's bit rate index. The last entry is not a
/// valid index.
const BITRATES: [u32; 16] = [
    0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
];

/// The sample rates an MPEG-1 frame header can name, by the header's sample
/// rate index. The last entry is not a valid index.
const RATES: [u32; 4] = [44_100, 48_000, 32_000, 0];

/// The bit rate in kilobits per second and the sample rate of every frame
/// of an MP3 file, in order.
///
/// The file may begin with an ID3 version 2 tag, which is skipped, and may
/// end with an ID3 version 1 tag, which is left out. Every frame must be an
/// MPEG-1 layer III frame that begins exactly where the previous one ends,
/// and the last must end exactly at the end of the file (or of the tag);
/// anything else panics, and the panic names the byte offset where the
/// chain of frames breaks.
pub fn mp3_frames(bytes: &[u8]) -> Vec<(u32, u32)> {
    let mut at = 0;
    if bytes.starts_with(b"ID3") && bytes.len() >= 10 {
        let size = bytes[6..10]
            .iter()
            .fold(0usize, |size, byte| (size << 7) | usize::from(*byte & 0x7f));
        at = 10 + size;
    }
    let end = if bytes.len() >= 128 && &bytes[bytes.len() - 128..bytes.len() - 125] == b"TAG" {
        bytes.len() - 128
    } else {
        bytes.len()
    };
    let mut frames = Vec::new();
    while at < end {
        assert!(
            at + 4 <= end,
            "the file ends inside a frame header at byte {at}"
        );
        let header = &bytes[at..at + 4];
        assert!(
            header[0] == 0xff && header[1] & 0xe0 == 0xe0,
            "no frame sync at byte {at}"
        );
        assert_eq!(header[1] & 0x18, 0x18, "not an MPEG-1 frame at byte {at}");
        assert_eq!(header[1] & 0x06, 0x02, "not a layer III frame at byte {at}");
        let bitrate = BITRATES[usize::from(header[2] >> 4)];
        let rate = RATES[usize::from((header[2] >> 2) & 3)];
        assert!(bitrate > 0 && rate > 0, "an invalid header at byte {at}");
        let padding = usize::from((header[2] >> 1) & 1);
        frames.push((bitrate, rate));
        at += 144 * bitrate as usize * 1000 / rate as usize + padding;
    }
    assert_eq!(at, end, "the last frame runs past the end of the file");
    frames
}
