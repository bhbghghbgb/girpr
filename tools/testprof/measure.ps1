# measure.ps1 — per-test wall/CPU/IO timing for one libtest binary
#
# Runs the binary with --test-threads=1 so that every "test <name> ... ok"
# line is a clean boundary between two tests, timestamps each boundary, and
# samples the child process's CPU time and I/O counters at the same instants.
# The delta between two boundaries is that test's wall time, its user+kernel
# CPU time, and its read/write operations and bytes.
#
#   wall - cpu  ==  the time the test spent BLOCKED (sleeps, file locks,
#                   fsync, connect timeouts, waiting for a child process)
#
# Windows notes, all three of which are easy to get wrong:
#   1. $p.StandardOutput is null until Start() has been called, and enabling
#      BeginOutputReadLine detaches it again -> capture the reader in between.
#   2. stderr must be drained asynchronously or a chatty test deadlocks
#      against a full pipe. The handler has to be attached from C#, because a
#      PowerShell script-block delegate needs a runspace and fails on the
#      thread-pool thread the event arrives on.
#   3. The child must be the test binary itself, not cmd.exe wrapping it:
#      cmd.exe's CPU time does not include its grandchild's.
#
# Usage:
#   .\measure.ps1 -Exe <path-to-test.exe> [-Threads 1] [-Csv <out.csv>] [-Quiet]

param(
  [Parameter(Mandatory = $true)][string]$Exe,
  [int]$Threads = 1,
  [string]$Csv = '',
  [switch]$Quiet
)

$ErrorActionPreference = 'Stop'

if (-not ('TestSink' -as [type])) {
  Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
using System.Diagnostics;

public static class IoCounters {
  [StructLayout(LayoutKind.Sequential)]
  public struct IOCounters {
    public ulong ReadOperationCount;
    public ulong WriteOperationCount;
    public ulong OtherOperationCount;
    public ulong ReadTransferCount;
    public ulong WriteTransferCount;
    public ulong OtherTransferCount;
  }
  [DllImport("kernel32.dll", SetLastError = true)]
  static extern bool GetProcessIoCounters(IntPtr hProcess, out IOCounters counters);
  public static IOCounters Read(Process p) {
    IOCounters c;
    try {
      if (!GetProcessIoCounters(p.Handle, out c)) { c = new IOCounters(); }
    } catch { c = new IOCounters(); }
    return c;
  }
}

public class TestSink {
  static readonly StringBuilder Buf = new StringBuilder();
  static readonly object Lock = new object();
  public static void Attach(Process p) {
    p.ErrorDataReceived += (sender, e) => {
      if (e.Data == null) return;
      lock (Lock) { Buf.AppendLine(e.Data); }
    };
    p.BeginErrorReadLine();
  }
  public static string Drain() { lock (Lock) { var s = Buf.ToString(); Buf.Clear(); return s; } }
}
'@
}

$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = $Exe
$psi.Arguments = if ($Threads -gt 0) { "--test-threads=$Threads" } else { "" }
$psi.UseShellExecute = $false
$psi.RedirectStandardOutput = $true
$psi.RedirectStandardError = $true
$psi.CreateNoWindow = $true

$p = New-Object System.Diagnostics.Process
$p.StartInfo = $psi

$sw = [System.Diagnostics.Stopwatch]::StartNew()
$null = $p.Start()
$stdout = $p.StandardOutput
[TestSink]::Attach($p)

$rows = New-Object System.Collections.Generic.List[object]
$prevWall = 0.0
$prevCpu = 0.0
$prevIo = [IoCounters]::Read($p)

while ($true) {
  $line = $stdout.ReadLine()
  if ($null -eq $line) { break }
  $t = $sw.Elapsed.TotalMilliseconds
  if ($line -match '^test (.+?) \.\.\. (ok|FAILED|ignored)$') {
    $name = $Matches[1]
    $verdict = $Matches[2]
    try { $cpu = $p.TotalProcessorTime.TotalMilliseconds } catch { $cpu = $prevCpu }
    if ($cpu -lt $prevCpu) { $cpu = $prevCpu }
    $io = [IoCounters]::Read($p)
    $rows.Add([pscustomobject]@{
      exe       = [System.IO.Path]::GetFileNameWithoutExtension($Exe)
      name      = $name
      verdict   = $verdict
      wall_ms   = [math]::Round($t - $prevWall, 1)
      cpu_ms    = [math]::Round($cpu - $prevCpu, 1)
      read_ops  = [int64]($io.ReadOperationCount - $prevIo.ReadOperationCount)
      read_kb   = [math]::Round(($io.ReadTransferCount - $prevIo.ReadTransferCount) / 1kb, 1)
      write_ops = [int64]($io.WriteOperationCount - $prevIo.WriteOperationCount)
      write_kb  = [math]::Round(($io.WriteTransferCount - $prevIo.WriteTransferCount) / 1kb, 1)
    })
    $prevWall = $t
    $prevCpu = $cpu
    $prevIo = $io
  }
}
$p.WaitForExit()
$sw.Stop()

$teardownMs = $sw.Elapsed.TotalMilliseconds - $prevWall
$totalCpu = 0.0
try { $totalCpu = $p.TotalProcessorTime.TotalMilliseconds } catch {}

if ($Csv -ne '') {
  $rows | Export-Csv -Path $Csv -NoTypeInformation -Encoding UTF8
} elseif (-not $Quiet) {
  $rows | Format-Table -AutoSize
}

$sum = ($rows | Measure-Object -Property wall_ms -Sum).Sum
$summary = [pscustomobject]@{
  exe       = [System.IO.Path]::GetFileNameWithoutExtension($Exe)
  threads   = $Threads
  tests     = $rows.Count
  total_ms  = [math]::Round($sw.Elapsed.TotalMilliseconds, 1)
  test_ms   = [math]::Round($sum, 1)
  cpu_ms    = [math]::Round($totalCpu, 1)
  teardown  = [math]::Round($teardownMs, 1)
  exit      = $p.ExitCode
}
$summary | ConvertTo-Json -Compress

if ($p.ExitCode -ne 0) {
  Write-Host "---- harness captured output (first 60 lines) ----"
  ([TestSink]::Drain() -split "`n" | Select-Object -First 60) | ForEach-Object { Write-Host $_ }
}
