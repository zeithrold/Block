param(
  [string]$Binary = 'D:\block-target\debug\block.exe',
  [string]$Seed = 'D:\block-src\target\e2e-pages-20261023',
  [string]$Driver = 'D:\block-tools\ui-drive3.exe',
  [string]$Output = 'D:\block-tools\evidence-e01-20261024'
)
# Run in an interactive scheduled task. Outputs are candidates until the local
# capture_assertions validator verifies title, dimensions and unique account text.
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $Output | Out-Null
$utf8 = New-Object System.Text.UTF8Encoding($false)
function Write-Utf8($Path, $Text) { [IO.File]::WriteAllText($Path, $Text, $utf8) }
Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class AccountCaptureWindow {
  [StructLayout(LayoutKind.Sequential)] public struct Rect { public int left,top,right,bottom; }
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out Rect r);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder b, int n);
}
'@
$app = $null
try {
  $accounts = @(Get-Content (Join-Path $Seed 'accounts.json') -Raw | ConvertFrom-Json)
  if($accounts.Count -ne 1 -or $accounts[0].provider -ne 'offline' -or $accounts[0].client_id -ne '' -or $accounts[0].credential_ref -ne '') {
    throw 'Expected exactly one credential-free offline seed identity'
  }
  $data = Join-Path $Output 'data'
  New-Item -ItemType Directory -Force -Path $data | Out-Null
  Copy-Item (Join-Path $Seed 'accounts.json') (Join-Path $data 'accounts.json') -Force
  $binaryHash = (Get-FileHash $Binary -Algorithm SHA256).Hash.ToLowerInvariant()
  $baseCommit = (git -C D:\block-src rev-parse HEAD).Trim()
  $buildInfo = Join-Path $Output 'build-info.txt'
  Start-Process -FilePath $Binary -ArgumentList '--build-info' -RedirectStandardOutput $buildInfo -Wait
  $identity = [IO.File]::ReadAllText($buildInfo) | ConvertFrom-Json
  foreach($theme in @('light','dark')) {
    Write-Utf8 (Join-Path $data 'preferences.json') (@{appearance=(Get-Culture).TextInfo.ToTitleCase($theme);onboarding_complete=$true;reduce_motion=$true} | ConvertTo-Json)
    $app = Start-Process -FilePath $Binary -ArgumentList '--data-dir',$data -PassThru
    $appId = $app.Id
    $batch = @"
waitwin pid=$appId timeout=20000
wait ms=800
click pid=$appId x=32 y=24
wait ms=400
click pid=$appId x=80 y=170
wait ms=1200
waitwin pid=$appId title="Accounts" timeout=10000
"@
    $batchPath = Join-Path $Output 'navigate.batch'
    Write-Utf8 $batchPath $batch
    & $Driver batch $batchPath $Output | Out-File (Join-Path $Output "navigate-$theme.log") -Encoding utf8
    if($LASTEXITCODE -ne 0) { throw "Accounts navigation failed for $theme" }
    $app.Refresh()
    $handle = $app.MainWindowHandle
    $title = New-Object Text.StringBuilder 256
    [AccountCaptureWindow]::GetWindowText($handle,$title,256) | Out-Null
    if($title.ToString() -ne 'Block · Accounts') { throw ('Wrong page: '+$title) }
    $rect = New-Object AccountCaptureWindow+Rect
    [AccountCaptureWindow]::GetClientRect($handle,[ref]$rect) | Out-Null
    $width = $rect.right-$rect.left
    $height = $rect.bottom-$rect.top
    # Exact viewport is independently checked locally before publication.
    Write-Utf8 $batchPath "shot pid=$appId title=`"Accounts`" name=accounts-$theme"
    & $Driver batch $batchPath $Output | Out-File (Join-Path $Output "shot-$theme.log") -Encoding utf8
    if($LASTEXITCODE -ne 0) { throw 'Window capture failed' }
    $receipt = @{
      schema_version=1; kind='real-data'; platform='windows-amd64'; theme=$theme;
      captured_at=[DateTime]::UtcNow.ToString('o'); binary_sha256=$binaryHash;
      source=@{commit=$identity.source_commit;source_sha256=$identity.source_sha256;checkout_base_commit=$baseCommit;note='Source identity comes from immutable --build-info; null means unavailable. Checkout base is not binary identity.'};
      build_info=[IO.File]::ReadAllText($buildInfo);
      window=@{title=$title.ToString();width=$width;height=$height;pid=$appId};
      method='Interactive scheduled task; ui-drive3 process-owned PrintWindow client capture; locally validated OCR before publication';
      data_state=('Ordinary persisted offline account '+$accounts[0].player_name+' copied from the historical seed; empty game catalog; disposable --data-dir; no --scenario or live credentials.');
      reduced_motion=$true;focus='Accounts menu item clicked; final native focus handle unknown';
      scroll='Fresh Accounts page; no scroll input; top Account details content asserted';
      trigger='Home app menu → Accounts menu item';
      required_text=@('Accounts',$accounts[0].player_name,'Account details','Offline Mode','Local identity; no online authorization')
    }
    Write-Utf8 (Join-Path $Output "accounts-$theme.candidate.json") ($receipt | ConvertTo-Json -Depth 10)
    Write-Utf8 $batchPath "close pid=$appId"
    & $Driver batch $batchPath $Output | Out-Null
    $app = $null
  }
  Write-Utf8 (Join-Path $Output 'done.txt') 'OK'
} catch {
  Write-Utf8 (Join-Path $Output 'done.txt') ('FAIL '+$_.Exception.Message)
  throw
} finally {
  if($app -and -not $app.HasExited) { $app.CloseMainWindow() | Out-Null; if(-not $app.WaitForExit(3000)) { $app.Kill() } }
}
