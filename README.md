<div align="center">

<img src="docs/assets/logo_text.svg" alt="Tao Logo" width="600" />

# Tao

**A pure Rust multimedia processing framework.**

[English](README.md) | [中文 (Chinese)](README_CN.md)

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
</div>

## What is Tao?

Tao (道) is a multimedia processing framework written in pure Rust, aiming to be a modern, memory-safe alternative to FFmpeg. It provides a comprehensive suite of tools and libraries for recording, converting, and streaming audio and video.

**WARNING**: Tao is currently in the early stages of development. APIs are subject to change.

## Design Goals

*   **Pure Rust**: Leveraging Rust's memory safety and concurrency features.
*   **Modular**: Components are split into crates (`tao-core`, `tao-codec`, `tao-format`, etc.).
*   **Performance**: Aiming for high performance comparable to C/C++ implementations.
*   **Compatibility**: Striving for feature parity with FFmpeg.

## Features

*   **Tao CLI**: Command-line tool similar to `ffmpeg`.
*   **Tao Probe**: Multimedia stream analyzer similar to `ffprobe`.
*   **Tao Play**: Simple media player similar to `ffplay`.
*   **Format Support**: Support for common containers (MP4, MKV, AVI, etc.).
*   **Codec Support**: Support for common codecs (H.264, AAC, etc.).

## Documentation

*   [Quick Start Guide](docs/quick_start.md)
*   [API Documentation](docs/api_docs.md)

## Contributing

We welcome contributions! Please see our [Contributing Guide](CONTRIBUTING.md).

## Credits & Inspiration

Tao is heavily inspired by and references the architecture and implementation of:
*   [FFmpeg](https://ffmpeg.org/)
*   [XVid](https://www.xvid.com/)
*   And many other open-source multimedia projects.

We thank the open-source community for their tremendous work in this field.

## AI Acknowledgment

This project utilizes various AI models, including Gemini, Claude, and Codex, for code generation and assistance. Approximately 99% of the work is automated by AI, with human supervision for process control and testing.

## Contact

For inquiries, please contact: **xjkuchao@gmail.com** (Subject: Tao Project).

## License

This project is licensed under the MIT License - see the [LICENSE-MIT](LICENSE-MIT) file for details.
