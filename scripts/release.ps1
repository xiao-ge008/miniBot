<#
.SYNOPSIS
  miniBot 一键版本管理与 GitHub 自动打包发布脚本
.DESCRIPTION
  自动同步更新 package.json、Cargo.toml、tauri.conf.json 三处版本号，
  创建 Git Commit 与 vX.Y.Z Tag 并推送到 GitHub，触发 GitHub Actions 自动编译打包
  Windows (.exe) 与 macOS (.dmg) 安装包并发布到 GitHub Releases 供用户客户端在线检测更新。
.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\scripts\release.ps1 patch "优化划词与截图体验"
  powershell -ExecutionPolicy Bypass -File .\scripts\release.ps1 minor "新增版本在线自动更新功能"
  powershell -ExecutionPolicy Bypass -File .\scripts\release.ps1 0.2.0 "发布 0.2.0 正式版"
#>
param(
    [Parameter(Position = 0)]
    [string]$VersionOrBump = "patch",

    [Parameter(Position = 1)]
    [string]$Message = "常规功能优化与体验改进"
)

$ErrorActionPreference = "Stop"
$RootDir = Split-Path -Parent $PSScriptRoot
Set-Location $RootDir

$PkgJsonPath   = Join-Path $RootDir "app\package.json"
$TauriConfPath = Join-Path $RootDir "app\src-tauri\tauri.conf.json"
$CargoTomlPath = Join-Path $RootDir "app\src-tauri\Cargo.toml"

# 1. 读取当前版本号
$tauriObj = Get-Content $TauriConfPath -Raw -Encoding UTF8 | ConvertFrom-Json
$curVer = $tauriObj.version.Trim()
$parts = $curVer -split '\.'
[int]$major = $parts[0]
[int]$minor = $parts[1]
[int]$patch = $parts[2]

# 2. 计算新版本号
switch ($VersionOrBump.ToLower()) {
    "patch" { $patch += 1; $newVer = "$major.$minor.$patch" }
    "minor" { $minor += 1; $patch = 0; $newVer = "$major.$minor.$patch" }
    "major" { $major += 1; $minor = 0; $patch = 0; $newVer = "$major.$minor.$patch" }
    default {
        $clean = $VersionOrBump.TrimStart('v', 'V')
        if ($clean -notmatch '^\d+\.\d+\.\d+$') {
            throw "无效的版本号参数: $VersionOrBump （请传入 patch / minor / major 或 x.y.z 格式版本号）"
        }
        $newVer = $clean
    }
}

Write-Host "====================================================" -ForegroundColor Cyan
Write-Host "  miniBot 版本发布工具: v$curVer -> v$newVer" -ForegroundColor Cyan
Write-Host "  更新说明: $Message" -ForegroundColor Cyan
Write-Host "====================================================" -ForegroundColor Cyan

# 3. 同步更新三处配置文件中的版本号
$utf8NoBom = New-Object System.Text.UTF8Encoding($false)

$pkgContent = [System.IO.File]::ReadAllText($PkgJsonPath, $utf8NoBom)
$pkgContent = [regex]::Replace($pkgContent, '"version"\s*:\s*"[^"]+"', "`"version`": `"$newVer`"", 1)
[System.IO.File]::WriteAllText($PkgJsonPath, $pkgContent, $utf8NoBom)

$tauriContent = [System.IO.File]::ReadAllText($TauriConfPath, $utf8NoBom)
$tauriContent = [regex]::Replace($tauriContent, '"version"\s*:\s*"[^"]+"', "`"version`": `"$newVer`"", 1)
[System.IO.File]::WriteAllText($TauriConfPath, $tauriContent, $utf8NoBom)

$cargoContent = [System.IO.File]::ReadAllText($CargoTomlPath, $utf8NoBom)
$cargoContent = [regex]::Replace($cargoContent, '(?m)^version\s*=\s*"[^"]+"', "version = `"$newVer`"", 1)
[System.IO.File]::WriteAllText($CargoTomlPath, $cargoContent, $utf8NoBom)

Write-Host "[1/3] 已同步更新 package.json / tauri.conf.json / Cargo.toml 版本号为 $newVer" -ForegroundColor Green

# 4. Git 提交并打 Tag
git add .
git commit -m "chore(release): v${newVer} - $Message"
git tag -f "v${newVer}" -m "miniBot v${newVer}: $Message"
Write-Host "[2/3] 已创建 Git 提交与标签 v${newVer}" -ForegroundColor Green

# 5. 推送到 GitHub 触发 Actions 自动打包与 Release 发布
git push origin main
git push origin "v$newVer" --force
Write-Host "[3/3] 已推送至 GitHub！GitHub Actions 正在云端自动打包 Windows (.exe) 与 macOS (.dmg) 并发布至 Releases！" -ForegroundColor Green
Write-Host "      查看打包进度: https://github.com/xiao-ge008/miniBot/actions" -ForegroundColor Yellow
Write-Host "      查看发布页面: https://github.com/xiao-ge008/miniBot/releases/tag/v$newVer" -ForegroundColor Yellow
