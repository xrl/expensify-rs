class ExpensifyCli < Formula
  desc "Command-line client for the Expensify Integration Server API"
  homepage "https://github.com/xrl/expensify-rs"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/xrl/expensify-rs/releases/download/expensify-cli-0.2.0/expensify-cli-0.2.0-aarch64-apple-darwin.tar.gz"
      sha256 "42238f0384a521ea212876b6f62e9b80f213fca14ad93aa860e81933055a9c6e"
    else
      odie "expensify-cli ships only Apple Silicon (arm64) macOS bottles. Build from source: " \
           "cargo install expensify-cli"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/xrl/expensify-rs/releases/download/expensify-cli-0.2.0/expensify-cli-0.2.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "532368dc858a8fe4dfc9a16ce4873f74adfed6bab9e734fc35bf30a4a6000f91"
    else
      url "https://github.com/xrl/expensify-rs/releases/download/expensify-cli-0.2.0/expensify-cli-0.2.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "b0a1901254ce87ad1c99f940bbd8aa4e6fd5e2105ab82c837f627274facd6b0f"
    end
  end

  def install
    bin.install "expensify"
  end

  test do
    assert_match "expensify #{version}", shell_output("#{bin}/expensify --version")
  end
end
