[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string] $OutputPath,
    [switch] $Force
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$output = [IO.Path]::GetFullPath($OutputPath)
if ($output.StartsWith($repository.TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Write the public snapshot outside the development checkout.'
}
if ([IO.Path]::GetExtension($output) -ne '.zip') {
    throw 'OutputPath must end in .zip.'
}
if ((Test-Path -LiteralPath $output) -and -not $Force) {
    throw 'The output already exists. Use -Force to replace it.'
}

Push-Location $repository
try {
    $changes = & git status --porcelain --untracked-files=normal
    if ($LASTEXITCODE -ne 0) { throw 'The source directory must be a Git repository.' }
    if ($changes) { throw 'Commit or remove pending source changes before exporting the latest snapshot.' }
    & node (Join-Path $PSScriptRoot 'check-public-source.mjs') --staged
    if ($LASTEXITCODE -ne 0) { throw 'Public-source checks failed.' }
    $tree = (& git rev-parse 'HEAD^{tree}').Trim()
    if ($LASTEXITCODE -ne 0) { throw 'The repository must contain a commit.' }
    $parent = [IO.Path]::GetDirectoryName($output)
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    # Archiving a tree includes tracked source only, without a commit identifier
    # in the ZIP comment, Git metadata, history, or ignored dependency caches.
    & git archive --format=zip "--output=$output" $tree
    if ($LASTEXITCODE -ne 0) { throw 'Git snapshot export failed.' }
    $hash = (Get-FileHash -LiteralPath $output -Algorithm SHA256).Hash.ToLowerInvariant()
    Write-Output "Source snapshot: $output"
    Write-Output "SHA-256: $hash"
    Write-Output 'Extract into a new directory and initialize a fresh repository for the first public commit.'
}
finally {
    Pop-Location
}
