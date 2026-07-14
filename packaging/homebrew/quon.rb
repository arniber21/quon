# frozen_string_literal: true

# Homebrew formula for Quon self-contained CLI binaries (issue #235).
#
# This formula downloads prebuilt GitHub Release archives. It must NOT
# depend_on "llvm@22" or "z3" — those are compile-time only; release binaries
# statically link MLIR/LLVM/Z3.
#
# Source of truth for SHA256s: regenerate via
#   ./scripts/generate-homebrew-formula.sh
# after a tagged release (or from local dist/ archives). Publish the result to
# the external tap `arniber21/homebrew-quon` (see packaging/homebrew/README.md).
#
# Placeholders below are filled by generate-homebrew-formula.sh:
#   __VERSION__ / __SHA256_*__

class Quon < Formula
  desc "MLIR-based optimizing compiler for quantum programs"
  homepage "https://github.com/arniber21/quon"
  license "Apache-2.0"
  version "__VERSION__"

  on_macos do
    on_arm do
      url "https://github.com/arniber21/quon/releases/download/v#{version}/quon-#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "__SHA256_AARCH64_APPLE_DARWIN__"
    end
    # Intel macOS archives are optional until a CI matrix job ships them.
    on_intel do
      url "https://github.com/arniber21/quon/releases/download/v#{version}/quon-#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "__SHA256_X86_64_APPLE_DARWIN__"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/arniber21/quon/releases/download/v#{version}/quon-#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "__SHA256_X86_64_UNKNOWN_LINUX_GNU__"
    end
    on_arm do
      url "https://github.com/arniber21/quon/releases/download/v#{version}/quon-#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "__SHA256_AARCH64_UNKNOWN_LINUX_GNU__"
    end
  end

  # Runtime: none beyond the OS. Do not add llvm@22 / z3 / mlir here.

  def install
    bin.install "quonc", "quonfmt", "quon_lsp", "quonlint"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/quonc --version")
    assert_match version.to_s, shell_output("#{bin}/quonfmt --version")
  end
end
