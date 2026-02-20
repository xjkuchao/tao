<div align="center">

<img src="docs/assets/logo_text.svg" alt="Tao Logo" width="600" />

# Tao

**A pure Rust multimedia processing framework.**

[English](README.md) | [中文 (Chinese)](README_CN.md)

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

</div>

## ⚠️ About This Experimental Project

**This is a highly ambitious, experimental AI-driven project!**

The core mission of this project is: **To build a highly complex multimedia framework from the ground up without a human writing a single line of code.** Using only human architecture steering and process flow control, the entire codebase is developed collaboratively by the latest AI programming tools (VSCode, Cursor, Antigravity, Claude Code, etc.) and state-of-the-art foundation models (Claude, Gemini, Codex, etc.).

**Why choose an Audio/Video Codec Framework?**
Through evaluation, we realized that implementing a robust multimedia framework like FFmpeg represents one of the most challenging software engineering realms. It requires dealing with complex mathematical algorithms, architectural designs, and clear, highly extensible structures. More importantly, it has a definitive, rigid benchmark: it must achieve **100% bit-exact, frame-by-frame, and sample-by-sample precision** when compared directly to the original FFmpeg.

Therefore, we have defined the ultimate goal of this project as: **An AI-powered, 100% pure Rust replication of FFmpeg's core codec pipelines from scratch.**

**Our Strict Implementation Constraints:**
We do NOT wrap or rely on any existing third-party codec implementations, filters, or external multimedia libraries. Everything—from container demuxers to low-level pixel transformations, motion compensation arrays, and entropy decoders—is **completely self-implemented and autonomously debugged by AI**.

Throughout the repository, we have preserved comprehensive working plans, debugging analysis, and AI thought processes in our commit history and `plans/` directories. You can trace exactly how different AI agents conceptualize architecture, deduce formulas, and resolve complex bugs. As AI's coding capabilities enter a period of explosive growth, we hope that this project, grown entirely through pure AI inference, provides a unique and hardcore perspective for future reflections on the evolution of software engineering powered by artificial intelligence.

## Design Goals

- **Pure Rust**: Leveraging Rust's memory safety and concurrency features.
- **Modular**: Components are split into crates (`tao-core`, `tao-codec`, `tao-format`, etc.).
- **Performance**: Aiming for high performance comparable to C/C++ implementations.
- **Compatibility**: Striving for feature parity with FFmpeg.

## Features

- **Tao CLI**: Command-line tool similar to `ffmpeg`.
- **Tao Probe**: Multimedia stream analyzer similar to `ffprobe`.
- **Tao Play**: Simple media player similar to `ffplay`.
- **Format Support**: Support for common containers (MP4, MKV, AVI, etc.).
- **Codec Support**: Support for common codecs (H.264, AAC, etc.).

## Documentation

- [Quick Start Guide](docs/quick_start.md)
- [API Documentation](docs/api_docs.md)

## Contributing

We welcome contributions! Please see our [Contributing Guide](CONTRIBUTING.md).

## Credits & Inspiration

Tao is heavily inspired by and references the architecture and implementation of:

- [FFmpeg](https://ffmpeg.org/)
- [XVid](https://www.xvid.com/)
- And many other open-source multimedia projects.

We thank the open-source community for their tremendous work in this field.

## Contact

For inquiries, please contact: **xjkuchao@gmail.com** (Subject: Tao Project).

## License

This project is licensed under the MIT License - see the [LICENSE-MIT](LICENSE-MIT) file for details.
