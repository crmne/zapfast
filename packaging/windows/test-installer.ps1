# Installs, updates and removes ZapFast's Windows packages on a CI runner,
# with stand-in program files: the Inno Setup installer, the MSI
# taking it over (as the updater runs it, and by hand), an MSI update, and
# uninstalling with ZapFast running and then without.
#
#   pwsh packaging/windows/test-installer.ps1
#
# Needs Inno Setup 6, WiX 5 with WixToolset.Util.wixext, and Ruby. Run from
# the repository root. Changes the current user's ZapFast install.

$ErrorActionPreference = "Stop"
$root = Join-Path $env:RUNNER_TEMP "zapfast-installer-test"
Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
New-Item -ItemType Directory $root | Out-Null

$innoKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{F2512314-384A-4002-9933-AB840FD01639}_is1"
$startMenu = Join-Path ([Environment]::GetFolderPath("Programs")) "ZapFast.lnk"
$desktop = Join-Path ([Environment]::GetFolderPath("Desktop")) "ZapFast.lnk"
$defaultFolder = Join-Path $env:LOCALAPPDATA "Programs\ZapFast"

function Check($condition, $what) {
  if (-not $condition) { throw "FAILED: $what" }
  Write-Host "ok: $what"
}

# Program files without a version resource, dated before their install as a
# release build is, which is what keeps Windows Installer from replacing them
# unless the package says to.
function Payload($version) {
  $dir = Join-Path $root "payload-$version"
  New-Item -ItemType Directory -Force $dir | Out-Null
  Set-Content -NoNewline -Path (Join-Path $dir "zapfast.exe") -Value "zapfast.exe $version"
  Copy-Item README.md, LICENSE, THIRD-PARTY-NOTICES.md, packaging\windows\zapfast-installer.txt $dir
  foreach ($name in "onnxruntime.dll", "onnxruntime-LICENSE.txt", "onnxruntime-ThirdPartyNotices.txt") {
    Set-Content -NoNewline -Path (Join-Path $dir $name) -Value "$name $version"
  }
  Get-ChildItem $dir | ForEach-Object { $_.LastWriteTime = (Get-Date).AddDays(-30) }
  $dir
}

function Inno($version, $arguments) {
  $iscc = (Get-Command iscc -ErrorAction SilentlyContinue).Source
  if (-not $iscc) { $iscc = "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe" }
  $binary = Join-Path (Payload "inno-$version") "zapfast.exe"
  & $iscc /Q "/DVersion=$version" /DArch=x86_64 "/DBinary=$binary" `
    "/DOutputDir=$root" packaging\windows\zapfast.iss
  if ($LASTEXITCODE -ne 0) { throw "ISCC failed" }
  $setup = Join-Path $root "zapfast-v$version-x86_64-pc-windows-msvc-setup.exe"
  $process = Start-Process $setup -ArgumentList $arguments -Wait -PassThru
  Check ($process.ExitCode -eq 0) "Inno Setup $version installs"
}

function Msi($version, $arch = "x64") {
  $package = Join-Path $root "zapfast-$version-$arch.msi"
  ruby packaging\windows\msi.rb (Payload "msi-$version-$arch") $package $version $arch | Out-Host
  if ($LASTEXITCODE -ne 0) { throw "MSI build failed" }
  $package
}

function Msiexec($arguments) {
  $log = Join-Path $root "msiexec-$([guid]::NewGuid()).log"
  $script:lastLog = $log
  $process = Start-Process msiexec.exe -ArgumentList "$arguments /l*v `"$log`"" -Wait -PassThru
  if ($process.ExitCode -notin 0, 3010) {
    Get-Content $log -Tail 60 -ErrorAction SilentlyContinue | Write-Host
  }
  $process.ExitCode
}

function Program($folder) {
  Get-Content -Raw (Join-Path $folder "zapfast.exe")
}

# What Settings > Apps lists for ZapFast: the MSI installs Windows Installer
# knows by ZapFast's UpgradeCode, and the Inno Setup entry.
function Entries {
  $installer = New-Object -ComObject WindowsInstaller.Installer
  $products = $installer.GetType().InvokeMember("RelatedProducts", "GetProperty", $null, $installer,
    @("{AC7FEAEC-54FA-4A71-95A5-532EE484B6A9}"))
  $count = @($products).Count
  if (Test-Path $innoKey) { $count += 1 }
  $count
}

# 1. The Inno Setup installer, with its desktop shortcut.
Inno "0.18.0" "/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /CURRENTUSER /TASKS=desktopicon"
Check (Test-Path (Join-Path $defaultFolder "unins000.exe")) "Inno installs its uninstaller"
Check (Test-Path $innoKey) "Inno registers in Settings > Apps"
Check (Test-Path $desktop) "Inno makes the desktop shortcut"
Check (Test-Path (Join-Path $defaultFolder "fastsapp.exe")) "Inno keeps the copy for old updaters"

# 2. The MSI, run the way the updater runs it.
$v1 = Msi "0.18.1"
$code = Msiexec "/i `"$v1`" /qn /norestart INSTALLFOLDER=`"$defaultFolder`""
Check ($code -in 0, 3010) "the MSI takes over the Inno install ($code)"
Check ((Program $defaultFolder) -eq "zapfast.exe msi-0.18.1-x64") "the program is replaced"
Check (Test-Path (Join-Path $defaultFolder "zapfast-installer.txt")) "the installer marker is in place"
Check (Test-Path (Join-Path $defaultFolder "onnxruntime.dll")) "the speech runtime is beside the program"
Check (-not (Test-Path (Join-Path $defaultFolder "unins000.exe"))) "Inno's uninstaller is gone"
Check (-not (Test-Path (Join-Path $defaultFolder "unins000.dat"))) "Inno's uninstall log is gone"
Check (-not (Test-Path $innoKey)) "Inno's Settings > Apps entry is gone"
Check (-not (Test-Path (Join-Path $defaultFolder "fastsapp.exe"))) "the copy kept for old updaters is gone"
Check ((Entries) -eq 1) "Settings > Apps lists ZapFast once"
Check (Test-Path $startMenu) "the Start menu shortcut is there"
Check (Test-Path $desktop) "the desktop shortcut is kept"

# 3. An MSI update run by hand finds the install on its own.
$v2 = Msi "0.18.2"
$code = Msiexec "/i `"$v2`" /qn /norestart"
Check ($code -in 0, 3010) "the next MSI updates in place ($code)"
Check ((Program $defaultFolder) -eq "zapfast.exe msi-0.18.2-x64") "the program is the new one"
Check ((Entries) -eq 1) "Settings > Apps still lists ZapFast once"
Check (Test-Path $desktop) "the desktop shortcut survives the update"

# 4. Uninstalling while ZapFast runs is refused, untouched.
$fake = Join-Path $root "running"
New-Item -ItemType Directory $fake | Out-Null
Copy-Item "$env:SystemRoot\System32\PING.EXE" (Join-Path $fake "zapfast.exe")
$running = Start-Process (Join-Path $fake "zapfast.exe") -ArgumentList "-n 600 127.0.0.1" -PassThru -WindowStyle Hidden
try {
  $code = Msiexec "/x `"$v2`" /qn /norestart"
  Check ($code -ne 0 -and $code -ne 3010) "uninstalling while ZapFast runs is refused ($code)"
  Check (Select-String -Quiet -Encoding unicode "ZapFast is running" $script:lastLog) "by the check for a running ZapFast"
  Check ((Program $defaultFolder) -eq "zapfast.exe msi-0.18.2-x64") "nothing was removed"
} finally {
  Stop-Process -Id $running.Id -Force
}

# 5. Uninstalling once it has quit removes everything.
$code = Msiexec "/x `"$v2`" /qn /norestart"
Check ($code -in 0, 3010) "uninstalling removes ZapFast ($code)"
Check (-not (Test-Path $defaultFolder)) "the program folder is gone"
Check (-not (Test-Path $startMenu)) "the Start menu shortcut is gone"
Check (-not (Test-Path $desktop)) "the desktop shortcut is gone"
Check ((Entries) -eq 0) "Settings > Apps no longer lists ZapFast"

# 6. An Inno install in a folder of its own, taken over by an MSI run by hand.
$custom = Join-Path $env:LOCALAPPDATA "ZapFast Elsewhere"
Inno "0.18.0" "/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /CURRENTUSER `"/DIR=$custom`""
$code = Msiexec "/i `"$v1`" /qn /norestart"
Check ($code -in 0, 3010) "a hand-run MSI takes over the Inno install ($code)"
Check ((Program $custom) -eq "zapfast.exe msi-0.18.1-x64") "it installs into Inno's folder"
Check (-not (Test-Path $defaultFolder)) "and not into the default one"
Check (-not (Test-Path $desktop)) "no desktop shortcut appears that Inno did not make"
$code = Msiexec "/x `"$v1`" /qn /norestart"
Check ($code -in 0, 3010) "and uninstalls ($code)"
Check (-not (Test-Path $custom)) "leaving no folder behind"

# 7. The Windows on ARM package builds.
Msi "0.18.1" "arm64" | Out-Null
Check $true "the arm64 MSI builds"
