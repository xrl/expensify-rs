class ExpensifyCli < Formula
  desc "Command-line client for the Expensify Integration Server API"
  homepage "https://github.com/xrl/expensify-rs"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/xrl/expensify-rs/releases/download/expensify-cli-0.1.0/expensify-cli-0.1.0-aarch64-apple-darwin.tar.gz"
      sha256 "5321a116fcfc7669f19152e6dd22e85bd0d4de4b7050286c2b4dbdd549dd6b4b"
    else
      odie "expensify-cli ships only Apple Silicon (arm64) macOS bottles. Build from source: " \
           "cargo install expensify-cli"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/xrl/expensify-rs/releases/download/expensify-cli-0.1.0/expensify-cli-0.1.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "556189910870d7c55a9aa25eb9b79027f8bff88e1d4616f75f3a4efadb087c49"
    else
      url "https://github.com/xrl/expensify-rs/releases/download/expensify-cli-0.1.0/expensify-cli-0.1.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "5e2a209bdddda512d3c983dd48cad12a42dd5876819a24cd61c8407ae0823afb"
    end
  end

  def install
    bin.install "expensify"
  end

  test do
    assert_match "expensify #{version}", shell_output("#{bin}/expensify --version")
  end
end
