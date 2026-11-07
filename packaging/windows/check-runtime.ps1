# Inspect the real executable, including delay-load imports, without running it.
# Hosted Windows runners already have the VC runtime, so launching there alone
# cannot detect a release that fails on a clean Windows installation.
param(
  [Parameter(Mandatory)][string]$Binary,
  [string]$Dumpbin
)

$ErrorActionPreference = 'Stop'
if (-not (Test-Path -LiteralPath $Binary -PathType Leaf)) {
  throw "Executable not found: $Binary"
}
if (-not $Dumpbin) {
  $command = Get-Command dumpbin.exe -ErrorAction SilentlyContinue
  if ($command) {
    $Dumpbin = $command.Source
  } else {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    if (-not (Test-Path -LiteralPath $vswhere)) {
      throw 'dumpbin.exe not found; install the Visual C++ build tools.'
    }
    $Dumpbin = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
      -find 'VC/Tools/MSVC/**/bin/Hostx64/x64/dumpbin.exe' | Select-Object -First 1
    if (-not $Dumpbin) { throw 'dumpbin.exe not found; install the Visual C++ build tools.' }
  }
}

$output = & $Dumpbin /nologo /dependents $Binary 2>&1
if ($LASTEXITCODE -ne 0) { throw "dumpbin failed for ${Binary}: $output" }
$imports = @($output | ForEach-Object {
  if ("$_" -match '^\s+([\w.-]+\.dll)\s*$') { $Matches[1] }
})
if ($imports.Count -eq 0) { throw "No DLL imports found in dumpbin output for $Binary" }
# The Universal CRT and legacy msvcrt.dll are Windows components. Versioned
# Visual C++ runtimes are not: neither the ZIP nor the per-user MSI supplies them.
$runtime = @($imports | Where-Object {
  $_ -match '^(vcruntime|msvcp|msvcr|concrt|vccorlib|vcomp)\d[^\s]*\.dll$'
})
if ($runtime.Count) {
  throw "${Binary} requires Visual C++ runtime DLLs: $($runtime -join ', '). Build with -C target-feature=+crt-static."
}
Write-Output "PASS: $Binary has no Visual C++ runtime DLL imports."
