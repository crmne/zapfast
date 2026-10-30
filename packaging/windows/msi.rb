# Builds ZapFast's Windows Installer package: native-packages' recipe for
# the windows-amd64 and windows-arm64 targets.
#
#   ruby packaging/windows/msi.rb PAYLOAD PACKAGE VERSION [ARCH]
#
# PAYLOAD holds zapfast.exe, README.md, LICENSE, THIRD-PARTY-NOTICES.md,
# zapfast-installer.txt and packaging/onnxruntime.py's onnxruntime.dll,
# license and notices. ARCH is x64 or arm64; without it, it comes from
# NATIVE_PACKAGES_TARGET. Needs WiX 5 and its Util extension:
#
#   dotnet tool install --global wix --version 5.0.2
#   wix extension add --global WixToolset.Util.wixext/5.0.2

payload, package, version, arch = ARGV
abort "usage: msi.rb PAYLOAD PACKAGE VERSION [x64|arm64]" unless payload && package && version
arch ||= { "windows-amd64" => "x64", "windows-arm64" => "arm64" }[ENV["NATIVE_PACKAGES_TARGET"]]
abort "unknown architecture: #{arch.inspect}" unless %w[x64 arm64].include?(arch)
abort "#{package} already exists" if File.exist?(package)

# Windows Installer versions are numeric: 1.2.3-beta.1 installs as 1.2.3.
numeric = version.split("-").first
success = system(
  "wix", "build", File.join(__dir__, "zapfast.wxs"),
  "-arch", arch,
  "-ext", "WixToolset.Util.wixext",
  "-d", "Version=#{numeric}",
  "-d", "SourceDir=#{File.expand_path(payload)}",
  # Only the declared package may appear next to it.
  "-pdbtype", "none",
  "-o", package
)
abort "wix was not found; install WiX 5 (see above)" if success.nil?
abort "MSI build failed (#{$?})" unless success
