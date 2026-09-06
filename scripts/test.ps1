. "$PSScriptRoot\common.ps1"
Set-Location -LiteralPath $script:RepositoryRoot

Invoke-NousCargo @("test", "--workspace", "--locked")
Invoke-NousCargo @("build", "-p", "nousd", "-p", "nous-provider-worker", "--locked")
Invoke-NousPython @("tests/e2e/kernel_e2e.py")
Invoke-NousPython @("tests/test_release_manifest.py", "-v")
$env:PYTHONPATH = Join-Path $script:RepositoryRoot "sdk\python\src"
Invoke-NousPython @("-m", "unittest", "discover", "-s", "sdk/python/tests", "-v")

$nativeOutput = Join-Path $script:RepositoryRoot "target\native-tests"
New-Item -ItemType Directory -Force -Path $nativeOutput | Out-Null
$cSdk = Join-Path $nativeOutput "nous-c-sdk-test.exe"
$micro = Join-Path $nativeOutput "nous-micro-test.exe"
Invoke-NousDeveloperCommand "cl /nologo /W4 /WX /std:c11 /I sdk/c/include sdk/c/src/nous_kernel.c sdk/c/tests/test_nous_kernel.c /Fe:$cSdk"
& $cSdk
if ($LASTEXITCODE -ne 0) { throw "C SDK test failed" }
Invoke-NousDeveloperCommand "cl /nologo /W4 /WX /std:c11 /I micro/include micro/src/nous_micro.c micro/tests/test_micro.c /Fe:$micro"
& $micro
if ($LASTEXITCODE -ne 0) { throw "Micro test failed" }
