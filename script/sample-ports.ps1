# Samples, every second until stopped, the TCP sockets on ephemeral ports and what holds them, for the
# Windows client job: a line per sample appended to the file $Samples, after a header of the range
# of ephemeral ports, the ranges excluded from it and the drives' free space. A test's connect that
# Windows fails with WSAENOBUFS (10055) prints the last line.
#
# A line: the ephemeral ports in use and their sockets by state; the non-paged pool; the five
# processes holding the most sockets, TimeWait aside, by name and ID; the five ports that the most
# sockets in TimeWait, which no process holds, went to, with the process listening there; the
# programs with the most processes; the free space of C: and D:. The table comes from
# GetExtendedTcpTable, in milliseconds where Get-NetTCPConnection takes seconds.
param([Parameter(Mandatory)] [string] $Samples)

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;

public static class PortSampler {
    [DllImport("iphlpapi.dll")]
    static extern uint GetExtendedTcpTable(IntPtr table, ref int size, bool sorted, int family, int tableClass, uint reserved);

    [StructLayout(LayoutKind.Sequential)]
    struct PerformanceInformation {
        public uint Size;
        public UIntPtr CommitTotal, CommitLimit, CommitPeak, PhysicalTotal, PhysicalAvailable,
            SystemCache, KernelTotal, KernelPaged, KernelNonpaged, PageSize;
        public uint HandleCount, ProcessCount, ThreadCount;
    }

    [DllImport("psapi.dll", SetLastError = true)]
    static extern bool GetPerformanceInfo(out PerformanceInformation information, uint size);

    const int TableOwnerPidAll = 5;
    const int Listen = 2;
    const int TimeWait = 11;
    static readonly string[] States = { "", "Closed", "Listen", "SynSent", "SynReceived",
        "Established", "FinWait1", "FinWait2", "CloseWait", "Closing", "LastAck", "TimeWait",
        "DeleteTcb" };

    struct Socket { public int State, LocalPort, RemotePort, Pid; }

    static int Port(int raw) { return ((raw & 0xff) << 8) | ((raw >> 8) & 0xff); }

    // The sockets of `family`, AF_INET or AF_INET6, whose rows are `row` bytes long with the
    // fields at these offsets.
    static IEnumerable<Socket> Table(int family, int row, int local, int remote, int state, int pid) {
        int size = 0;
        GetExtendedTcpTable(IntPtr.Zero, ref size, false, family, TableOwnerPidAll, 0);
        for (int attempt = 0; attempt < 5; attempt++) {
            size += 64 * row;
            IntPtr table = Marshal.AllocHGlobal(size);
            try {
                uint result = GetExtendedTcpTable(table, ref size, false, family, TableOwnerPidAll, 0);
                if (result == 122) continue; // ERROR_INSUFFICIENT_BUFFER: the table grew
                if (result != 0) return new Socket[0];
                int count = Marshal.ReadInt32(table);
                var sockets = new Socket[count];
                for (int i = 0; i < count; i++) {
                    IntPtr at = table + 4 + i * row;
                    sockets[i] = new Socket {
                        LocalPort = Port(Marshal.ReadInt32(at + local)),
                        RemotePort = Port(Marshal.ReadInt32(at + remote)),
                        State = Marshal.ReadInt32(at + state),
                        Pid = Marshal.ReadInt32(at + pid),
                    };
                }
                return sockets;
            } finally {
                Marshal.FreeHGlobal(table);
            }
        }
        return new Socket[0];
    }

    public static string Sample(int start) {
        // MIB_TCPROW_OWNER_PID and MIB_TCP6ROW_OWNER_PID.
        var sockets = Table(2, 24, 8, 16, 0, 20).Concat(Table(23, 56, 20, 44, 48, 52)).ToList();
        var names = new Dictionary<int, string>();
        var programs = new Dictionary<string, int>();
        foreach (var process in Process.GetProcesses()) {
            names[process.Id] = process.ProcessName;
            int count;
            programs.TryGetValue(process.ProcessName, out count);
            programs[process.ProcessName] = count + 1;
            process.Dispose();
        }
        Func<int, string> name = pid => {
            string found;
            return names.TryGetValue(pid, out found) ? found + " " + pid : "gone " + pid;
        };
        var listeners = new Dictionary<int, int>();
        foreach (var socket in sockets.Where(s => s.State == Listen)) listeners[socket.LocalPort] = socket.Pid;
        var used = sockets.Where(s => s.LocalPort >= start && s.State != Listen).ToList();
        int ports = used.Select(s => s.LocalPort).Distinct().Count();
        string states = string.Join(" ", used.GroupBy(s => s.State).OrderByDescending(g => g.Count())
            .Select(g => (g.Key < States.Length ? States[g.Key] : g.Key.ToString()) + "=" + g.Count()));
        string owners = string.Join(" ", used.Where(s => s.State != TimeWait).GroupBy(s => s.Pid)
            .OrderByDescending(g => g.Count()).Take(5).Select(g => name(g.Key) + "=" + g.Count()));
        string waits = string.Join(" ", used.Where(s => s.State == TimeWait).GroupBy(s => s.RemotePort)
            .OrderByDescending(g => g.Count()).Take(5).Select(g => {
                int pid;
                string to = listeners.TryGetValue(g.Key, out pid) ? " (" + name(pid) + ")" : "";
                return g.Key + to + "=" + g.Count();
            }));
        string most = string.Join(" ", programs.OrderByDescending(p => p.Value).Take(4)
            .Select(p => p.Key + "=" + p.Value));
        PerformanceInformation information;
        long pool = GetPerformanceInfo(out information, (uint)Marshal.SizeOf(typeof(PerformanceInformation)))
            ? (long)((ulong)information.KernelNonpaged * (ulong)information.PageSize >> 20) : -1;
        string free = string.Join(" ", new[] { "C", "D" }.Select(drive => {
            try { return drive + ": " + (new DriveInfo(drive).AvailableFreeSpace >> 30) + " GB"; }
            catch (Exception) { return drive + ": gone"; }
        }));
        return DateTime.Now.ToString("HH:mm:ss") + ": " + ports + " ports in use, " + states +
            "; non-paged pool " + pool + " MB; most: " + owners + "; TimeWait to: " + waits +
            "; processes: " + most + "; free: " + free;
    }
}
'@

$range = Get-NetTCPSetting -SettingName Internet
$start = $range.DynamicPortRangeStartPort
$excluded = (netsh interface ipv4 show excludedportrange protocol=tcp |
  Select-String -Pattern '^\s*(\d+)\s+(\d+)' |
  ForEach-Object { "$($_.Matches[0].Groups[1].Value)-$($_.Matches[0].Groups[2].Value)" }) -join ' '
"ephemeral ports $start-$($start + $range.DynamicPortRangeNumberOfPorts - 1); excluded: $excluded" |
  Add-Content -Path $Samples
"drives: " + ((Get-PSDrive -PSProvider FileSystem -Name C, D | ForEach-Object {
  "$($_.Name): $([int]($_.Free / 1GB)) GB free of $([int](($_.Used + $_.Free) / 1GB)) GB"
}) -join ', ') | Add-Content -Path $Samples
while ($true) {
  $began = Get-Date
  try {
    [PortSampler]::Sample($start) | Add-Content -Path $Samples
  } catch {
    "$(Get-Date -Format HH:mm:ss): no sample: $_" | Add-Content -Path $Samples
  }
  $left = 1000 - ((Get-Date) - $began).TotalMilliseconds
  if ($left -gt 0) { Start-Sleep -Milliseconds $left }
}
