# Exercise the checker without requiring Visual Studio or a compiled app.
$ErrorActionPreference = 'Stop'
$checker = Join-Path $PSScriptRoot 'check-runtime.ps1'
function Invoke-TestDumpbin {
  $global:LASTEXITCODE = $global:ZapRuntimeTestExit
  $global:ZapRuntimeTestOutput
}
function Check($name, $output, $exitCode, $expectedError) {
  $global:ZapRuntimeTestOutput = $output
  $global:ZapRuntimeTestExit = $exitCode
  $failure = $null
  try {
    & $checker -Binary $PSCommandPath -Dumpbin Invoke-TestDumpbin | Out-Null
  } catch { $failure = $_.Exception.Message }
  if ($expectedError) {
    if (-not $failure -or $failure -notlike "*$expectedError*") {
      throw "${name}: expected '$expectedError', got '$failure'"
    }
  } elseif ($failure) { throw "${name}: $failure" }
  Write-Output "PASS: $name"
}

Check 'Windows system libraries' @('    KERNEL32.dll', '    msvcrt.dll', '    ucrtbase.dll',
  '    api-ms-win-crt-runtime-l1-1-0.dll') 0 $null
foreach ($runtime in @('VCRUNTIME140.dll', 'vcruntime140_1.dll', 'MSVCP140.dll',
    'msvcp140_atomic_wait.dll', 'CONCRT140.dll', 'VCCORLIB140.dll', 'MSVCR120.dll')) {
  Check $runtime @('    KERNEL32.dll', "    $runtime") 0 'requires Visual C++ runtime DLLs'
}
Check 'Delay-loaded runtime' @('  Image has the following dependencies:', '    KERNEL32.dll',
  '  Image has the following delay load dependencies:', '    VCRUNTIME140.dll') 0 'requires Visual C++ runtime DLLs'
Check 'Tool failure' @('    KERNEL32.dll') 1 'dumpbin failed'
Check 'Unexpected tool output' @('File Type: EXECUTABLE IMAGE') 0 'No DLL imports found'
try {
  & $checker -Binary (Join-Path $PSScriptRoot 'nonexistent-runtime-test.exe') -Dumpbin Invoke-TestDumpbin
  throw 'Missing executable was accepted'
} catch {
  if ($_.Exception.Message -notlike 'Executable not found:*') { throw }
}
Write-Output 'PASS: missing executable'
Remove-Variable ZapRuntimeTestOutput, ZapRuntimeTestExit -Scope Global
