param([int]$BrowserPid)
$ErrorActionPreference = 'Stop'

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Threading;
using System.Threading.Tasks;

public static class WindowMonitor {
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window, out int pid);
    [DllImport("user32.dll")] static extern bool IsIconic(IntPtr window);
    [DllImport("dwmapi.dll")] static extern int DwmGetWindowAttribute(IntPtr window, int attribute, out int value, int size);

    public static void Watch(int browserPid) {
        int matched = 0, noForeground = 0, otherForeground = 0, unavailable = 0;
        var stop = Task.Run(() => Console.ReadLine());
        do {
            var window = GetForegroundWindow();
            int pid, cloaked;
            GetWindowThreadProcessId(window, out pid);
            if (window == IntPtr.Zero) {
                noForeground++;
            } else if (pid != browserPid) {
                otherForeground++;
            } else if (!IsIconic(window) &&
                DwmGetWindowAttribute(window, 14, out cloaked, sizeof(int)) == 0 && cloaked == 0) {
                matched++;
            } else {
                unavailable++;
            }
            if (matched + noForeground + otherForeground + unavailable == 1) Console.WriteLine("sampling");
            Thread.Sleep(10);
        } while (!stop.IsCompleted);
        Console.WriteLine("{{\"matched\":{0},\"noForeground\":{1},\"otherForeground\":{2},\"unavailable\":{3}}}",
            matched, noForeground, otherForeground, unavailable);
    }
}
'@
[WindowMonitor]::Watch($BrowserPid)
