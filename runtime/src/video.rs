use anyhow::{Context, Result, ensure};
use ffmpeg_next as ff;
use std::path::Path;

/// Native MPEG-4 encoder. No ffmpeg command, Python, OpenCV or helper process.
pub struct Encoder {
    output: ff::format::context::Output,
    encoder: ff::encoder::Video,
    scaler: ff::software::scaling::Context,
    width: usize,
    height: usize,
    time_base: ff::Rational,
    pub count: u64,
}
impl Encoder {
    pub fn new(path: &Path, width: usize, height: usize, fps: f64) -> Result<Self> {
        ff::init()?;
        let mut output = ff::format::output(path)?;
        let codec = ff::encoder::find(ff::codec::Id::MPEG4)
            .context("MPEG-4 encoder missing from native runtime")?;
        let mut encoder = ff::codec::context::Context::new_with_codec(codec)
            .encoder()
            .video()?;
        let rate = (fps * 1000.0).round() as i32;
        ensure!(
            fps.is_finite() && (1.0..=60.0).contains(&fps),
            "FPS must be 1–60"
        );
        let time_base = ff::Rational(1000, rate);
        encoder.set_width(width as u32);
        encoder.set_height(height as u32);
        encoder.set_format(ff::format::Pixel::YUV420P);
        encoder.set_time_base(time_base);
        encoder.set_frame_rate(Some(ff::Rational(rate, 1000)));
        encoder.set_bit_rate(4_000_000);
        encoder.set_max_b_frames(0);
        if output
            .format()
            .flags()
            .contains(ff::format::Flags::GLOBAL_HEADER)
        {
            encoder.set_flags(ff::codec::Flags::GLOBAL_HEADER);
        }
        let encoder = encoder.open()?;
        {
            let mut stream = output.add_stream(codec)?;
            stream.set_time_base(time_base);
            stream.set_parameters(&encoder);
        }
        output.write_header()?;
        let scaler = ff::software::scaling::Context::get(
            ff::format::Pixel::RGB24,
            width as u32,
            height as u32,
            ff::format::Pixel::YUV420P,
            width as u32,
            height as u32,
            ff::software::scaling::Flags::BILINEAR,
        )?;
        Ok(Self {
            output,
            encoder,
            scaler,
            width,
            height,
            time_base,
            count: 0,
        })
    }
    pub fn append(&mut self, rgb: &[u8]) -> Result<()> {
        ensure!(
            rgb.len() == self.width * self.height * 3,
            "Invalid RGB frame"
        );
        let mut source = ff::frame::Video::new(
            ff::format::Pixel::RGB24,
            self.width as u32,
            self.height as u32,
        );
        let stride = source.stride(0);
        for (y, row) in rgb.chunks_exact(self.width * 3).enumerate() {
            source.data_mut(0)[y * stride..y * stride + row.len()].copy_from_slice(row);
        }
        let mut frame = ff::frame::Video::empty();
        self.scaler.run(&source, &mut frame)?;
        frame.set_pts(Some(self.count as i64));
        self.encoder.send_frame(&frame)?;
        self.drain()?;
        self.count += 1;
        Ok(())
    }
    fn drain(&mut self) -> Result<()> {
        let mut packet = ff::Packet::empty();
        loop {
            match self.encoder.receive_packet(&mut packet) {
                Ok(()) => {
                    packet.set_stream(0);
                    packet.set_duration(1);
                    packet.rescale_ts(
                        self.time_base,
                        self.output.stream(0).context("Video stream")?.time_base(),
                    );
                    packet.write_interleaved(&mut self.output)?;
                }
                Err(ff::Error::Eof) => break,
                Err(ff::Error::Other { errno }) if errno == ff::error::EAGAIN => break,
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }
    pub fn finish(mut self) -> Result<u64> {
        self.encoder.send_eof()?;
        self.drain()?;
        self.output.write_trailer()?;
        Ok(self.count)
    }
}

pub struct Camera {
    input: ff::format::context::Input,
    decoder: ff::decoder::Video,
    scaler: ff::software::scaling::Context,
    stream: usize,
}
impl Camera {
    pub fn open(source: &serde_json::Value, fps: f64) -> Result<Self> {
        ff::init()?;
        ff::device::register_all();
        let input = if let Some(index) = source.as_u64() {
            let mut options = ff::Dictionary::new();
            options.set("video_size", "640x480");
            options.set("framerate", &fps.to_string());
            options.set("pixel_format", "uyvy422");
            let format = unsafe {
                let p = ff::ffi::av_find_input_format(c"avfoundation".as_ptr());
                ensure!(!p.is_null(), "AVFoundation camera input missing");
                ff::format::Input::wrap(p as *mut _)
            };
            match ff::format::open_with(
                &format!("{index}:none"),
                &ff::Format::Input(format),
                options,
            )? {
                ff::format::context::Context::Input(i) => i,
                _ => anyhow::bail!("Camera input required"),
            }
        } else {
            ff::format::input(
                &source
                    .as_str()
                    .context("Camera must be an index or video path")?,
            )?
        };
        let stream = input
            .streams()
            .best(ff::media::Type::Video)
            .context("No camera video stream")?;
        let index = stream.index();
        let decoder = ff::codec::context::Context::from_parameters(stream.parameters())?
            .decoder()
            .video()?;
        let scaler = ff::software::scaling::Context::get(
            decoder.format(),
            decoder.width(),
            decoder.height(),
            ff::format::Pixel::RGB24,
            640,
            480,
            ff::software::scaling::Flags::BILINEAR,
        )?;
        Ok(Self {
            input,
            decoder,
            scaler,
            stream: index,
        })
    }
    pub fn rgb(&mut self) -> Result<Vec<u8>> {
        let mut decoded = ff::frame::Video::empty();
        loop {
            match self.decoder.receive_frame(&mut decoded) {
                Ok(()) => {
                    let mut rgb = ff::frame::Video::empty();
                    self.scaler.run(&decoded, &mut rgb)?;
                    let mut pixels = Vec::with_capacity(640 * 480 * 3);
                    for y in 0..480 {
                        pixels.extend(&rgb.data(0)[y * rgb.stride(0)..y * rgb.stride(0) + 640 * 3]);
                    }
                    return Ok(pixels);
                }
                Err(ff::Error::Other { errno }) if errno == ff::error::EAGAIN => {}
                Err(e) => return Err(e.into()),
            }
            let mut packet = ff::Packet::empty();
            match packet.read(&mut self.input) {
                Ok(()) => {}
                Err(ff::Error::Eof) => {
                    self.decoder.send_eof()?;
                    continue;
                }
                Err(e) => return Err(e.into()),
            }
            if packet.stream() == self.stream {
                self.decoder.send_packet(&packet)?;
            }
        }
    }
}
