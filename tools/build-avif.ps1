param(
    [string]$Nasm = 'nasm',
    [int]$Jobs = 4
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$work = Join-Path $repo 'target/avif-prebuilt'
$destination = Join-Path $repo 'vendor/avif/native/x86_64-pc-windows-msvc'
$version = '0.17.2+libaom.3.11.0'
$checksum = '7c4fce8aaf0c2d8534529b0698ed98ec1da8d35319588d20b0d79ce6446e4194'
$assembler = (Get-Command $Nasm -ErrorAction Stop).Source
$assemblerVersion = & $assembler -v
if ($LASTEXITCODE -ne 0 -or $assemblerVersion -notmatch 'NASM version 2\.') {
    throw 'Rebuilding libaom requires NASM 2.x. Pass its executable with -Nasm.'
}
if ($Jobs -lt 1) { throw 'Jobs must be positive.' }
New-Item -ItemType Directory -Force $work, $destination | Out-Null
$archive = Join-Path $work "libaom-sys-$version.crate"
if (-not (Test-Path -LiteralPath $archive)) {
    Invoke-WebRequest "https://static.crates.io/crates/libaom-sys/libaom-sys-$version.crate" -OutFile $archive
}
if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $checksum) {
    throw 'libaom source checksum mismatch.'
}
tar -xf $archive -C $work
if ($LASTEXITCODE -ne 0) { throw 'Source extraction failed.' }
$source = Join-Path $work "libaom-sys-$version/vendor"
$build = Join-Path $work 'build'
cmake -S $source -B $build -A x64 "-DCMAKE_ASM_NASM_COMPILER=$assembler" `
    -DBUILD_SHARED_LIBS=0 -DCONFIG_AV1_ENCODER=0 -DCONFIG_AV1_DECODER=1 `
    -DENABLE_DOCS=0 -DENABLE_EXAMPLES=0 -DENABLE_TESTDATA=0 -DENABLE_TESTS=0 -DENABLE_TOOLS=0
if ($LASTEXITCODE -ne 0) { throw 'CMake configuration failed.' }
cmake --build $build --config Release --target aom --parallel $Jobs
if ($LASTEXITCODE -ne 0) { throw 'Native build failed.' }
$library = Join-Path $destination 'aom.lib'
Copy-Item -LiteralPath (Join-Path $build 'Release/aom.lib') -Destination $library -Force
$digest = (Get-FileHash -LiteralPath $library -Algorithm SHA256).Hash.ToLowerInvariant()
[IO.File]::WriteAllText((Join-Path $destination 'aom.lib.sha256'), "$digest  aom.lib`n")
Write-Output "Updated $library ($digest)"
