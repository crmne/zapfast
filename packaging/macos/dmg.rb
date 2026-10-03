# frozen_string_literal: true

# native-packages supplies an owned copy, already signed when Apple credentials
# are configured. Keep it unchanged and package only this input.
require "fileutils"
require "tmpdir"

payload, output = ARGV
abort "usage: dmg.rb PAYLOAD OUTPUT.dmg" unless payload && output
abort "output already exists: #{output}" if File.exist?(output)
Dir.mktmpdir("zapfast-dmg-") do |directory|
  FileUtils.cp_r(File.join(payload, "."), directory, preserve: true)
  File.symlink("/Applications", File.join(directory, "Applications"))
  # hdiutil sizes an image from -srcfolder by estimate, and on GitHub's macOS
  # runners that estimate has come up short ("No space left on device"), so
  # give it the payload's size with room to spare, and try again if it fails.
  kilobytes = Integer(`du -sk "#{directory}"`.split.first)
  megabytes = kilobytes * 13 / 10 / 1024 + 64
  created = 3.times.any? do |attempt|
    FileUtils.rm_f(output)
    sleep(5 * attempt)
    system("hdiutil", "create", "-volname", "ZapFast", "-srcfolder", directory,
      "-size", "#{megabytes}m", "-format", "UDZO", output)
  end
  unless created
    system("df", "-h", Dir.tmpdir, File.dirname(File.expand_path(output)))
    abort "DMG creation failed"
  end
  abort "DMG verification failed" unless system("hdiutil", "verify", output)
end
