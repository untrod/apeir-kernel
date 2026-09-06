. "$PSScriptRoot\common.ps1"
Set-Location -LiteralPath $script:RepositoryRoot
Invoke-NousCargo @("build", "--release", "--locked", "-p", "nousd", "-p", "nous-provider-worker", "-p", "nous-cli")

$distribution = Join-Path $script:RepositoryRoot "target\distribution"
New-Item -ItemType Directory -Force -Path $distribution | Out-Null
$names = @("nousd.exe", "nous-provider-worker.exe", "nous.exe")
foreach ($name in $names) {
    Copy-Item -LiteralPath (Join-Path $script:RepositoryRoot "target\release\$name") -Destination $distribution -Force
}
$supportFiles = @(
    "LICENSE",
    "NOTICE",
    "THIRD_PARTY_NOTICES",
    "README.md",
    "README.zh-CN.md",
    "DISTRIBUTION.md"
)
foreach ($name in $supportFiles) {
    Copy-Item -LiteralPath (Join-Path $script:RepositoryRoot $name) -Destination $distribution -Force
}
Copy-Item -LiteralPath (Join-Path $script:RepositoryRoot "spec\distribution\kernel-release-manifest-v1.schema.json") `
    -Destination (Join-Path $distribution "kernel-release-manifest-v1.schema.json") -Force
$artifacts = $names | ForEach-Object { Join-Path $distribution $_ }
& python (Join-Path $script:RepositoryRoot "scripts\write_release_manifest.py") `
    --output (Join-Path $distribution "nous-kernel-release.json") `
    --target ((& rustc -vV | Select-String '^host:').Line.Substring("host:".Length).Trim()) `
    --artifact "daemon=$($artifacts[0])" `
    --artifact "provider-worker=$($artifacts[1])" `
    --artifact "cli=$($artifacts[2])"
if ($LASTEXITCODE -ne 0) { throw "Kernel release manifest generation failed" }

$checksumNames = $names + $supportFiles + @(
    "kernel-release-manifest-v1.schema.json",
    "nous-kernel-release.json"
)
$checksumPaths = $checksumNames | ForEach-Object { Join-Path $distribution $_ }
Get-FileHash -Algorithm SHA256 -LiteralPath $checksumPaths |
    Select-Object @{Name="file";Expression={Split-Path -Leaf $_.Path}}, @{Name="sha256";Expression={$_.Hash}} |
    ConvertTo-Json | Set-Content -Encoding utf8 (Join-Path $distribution "SHA256.json")

$release = Get-Content -LiteralPath (Join-Path $distribution "nous-kernel-release.json") -Raw | ConvertFrom-Json
$packages = Join-Path $script:RepositoryRoot "target\packages"
New-Item -ItemType Directory -Force -Path $packages | Out-Null
$archive = Join-Path $packages "nous-kernel-$($release.version)-$($release.target).zip"
if (Test-Path -LiteralPath $archive) {
    Remove-Item -LiteralPath $archive -Force
}
Compress-Archive -Path (Join-Path $distribution "*") -DestinationPath $archive -CompressionLevel Optimal
$archiveHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash
"$archiveHash  $(Split-Path -Leaf $archive)" | Set-Content -Encoding ascii "$archive.sha256"
Write-Output "Portable Kernel bundle written to $distribution"
Write-Output "Portable Kernel archive written to $archive"
Write-Output "Portable Kernel archive checksum written to $archive.sha256"
