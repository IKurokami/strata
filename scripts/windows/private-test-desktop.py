#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Run selected Windows tests on a private desktop, never the interactive desktop.

Pass an exact Cargo test executable, not a normal application binary. This isolates
windows and profiles, but not the window station's clipboard: use only selections
which do not read or change the clipboard. Failure to isolate is fatal.
"""
import argparse
import ctypes as c
from ctypes import wintypes as w
import msvcrt
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time


class Startup(c.Structure):
    _fields_=[('cb',w.DWORD),('reserved',w.LPWSTR),('desktop',w.LPWSTR),('title',w.LPWSTR),
              ('x',w.DWORD),('y',w.DWORD),('width',w.DWORD),('height',w.DWORD),
              ('chars_x',w.DWORD),('chars_y',w.DWORD),('fill',w.DWORD),('flags',w.DWORD),
              ('show',w.WORD),('reserved_count',w.WORD),('reserved_bytes',c.c_void_p),
              ('stdin',w.HANDLE),('stdout',w.HANDLE),('stderr',w.HANDLE)]
class Process(c.Structure):
    _fields_=[('process',w.HANDLE),('thread',w.HANDLE),('pid',w.DWORD),('tid',w.DWORD)]
class Limits(c.Structure):
    _fields_=[('process_time',c.c_int64),('job_time',c.c_int64),('flags',w.DWORD),
              ('minimum',c.c_size_t),('maximum',c.c_size_t),('active',w.DWORD),
              ('affinity',c.c_size_t),('priority',w.DWORD),('scheduling',w.DWORD)]
class IoCounters(c.Structure):
    _fields_=[(name,c.c_uint64) for name in ('read','write','other','read_bytes','write_bytes','other_bytes')]
class Extended(c.Structure):
    _fields_=[('basic',Limits),('io',IoCounters),('process_memory',c.c_size_t),
              ('job_memory',c.c_size_t),('peak_process',c.c_size_t),('peak_job',c.c_size_t)]


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--log',type=Path,required=True)
    parser.add_argument('--timeout',type=int,default=900)
    parser.add_argument('command',nargs=argparse.REMAINDER)
    args=parser.parse_args()
    if sys.platform!='win32':
        parser.error('Private Win32 desktops require native Windows Python')
    command=args.command
    if command[:1]==['--']:command=command[1:]
    if not command or not Path(command[0]).is_file():
        parser.error('An explicit test executable is required')
    kernel=c.WinDLL('kernel32',use_last_error=True)
    user=c.WinDLL('user32',use_last_error=True)
    kernel.CreateProcessW.argtypes=[w.LPCWSTR,w.LPWSTR,c.c_void_p,c.c_void_p,w.BOOL,w.DWORD,c.c_void_p,w.LPCWSTR,c.POINTER(Startup),c.POINTER(Process)]
    kernel.CreateProcessW.restype=w.BOOL
    kernel.CreateJobObjectW.argtypes=[c.c_void_p,w.LPCWSTR];kernel.CreateJobObjectW.restype=w.HANDLE
    kernel.SetInformationJobObject.argtypes=[w.HANDLE,c.c_int,c.c_void_p,w.DWORD]
    kernel.AssignProcessToJobObject.argtypes=[w.HANDLE,w.HANDLE]
    kernel.ResumeThread.argtypes=[w.HANDLE];kernel.ResumeThread.restype=w.DWORD
    kernel.GetExitCodeProcess.argtypes=[w.HANDLE,c.POINTER(w.DWORD)]
    kernel.TerminateProcess.argtypes=[w.HANDLE,w.UINT]
    kernel.CloseHandle.argtypes=[w.HANDLE]
    user.GetProcessWindowStation.restype=w.HANDLE
    user.GetUserObjectInformationW.argtypes=[w.HANDLE,c.c_int,c.c_void_p,w.DWORD,c.POINTER(w.DWORD)]
    user.CreateDesktopW.argtypes=[w.LPCWSTR,w.LPCWSTR,c.c_void_p,w.DWORD,w.DWORD,c.c_void_p]
    user.CreateDesktopW.restype=w.HANDLE;user.CloseDesktop.argtypes=[w.HANDLE]
    station=c.create_unicode_buffer(1024);needed=w.DWORD()
    if not user.GetUserObjectInformationW(user.GetProcessWindowStation(),2,station,c.sizeof(station),c.byref(needed)):raise c.WinError(c.get_last_error())
    name=f'StrataTests-{os.getpid()}-{time.time_ns()}'
    desktop=user.CreateDesktopW(name,None,None,0,0x1ff,None)
    if not desktop:raise c.WinError(c.get_last_error())
    job=kernel.CreateJobObjectW(None,None);process=Process()
    try:
        if not job:raise c.WinError(c.get_last_error())
        limits=Extended();limits.basic.flags=0x2000
        if not kernel.SetInformationJobObject(job,9,c.byref(limits),c.sizeof(limits)):raise c.WinError(c.get_last_error())
        args.log.parent.mkdir(parents=True,exist_ok=True)
        with tempfile.TemporaryDirectory(prefix='strata-test-profile-') as temporary,args.log.open('wb') as log:
            profile=Path(temporary)
            for key,part in [('HOME','home'),('USERPROFILE','home'),('LOCALAPPDATA','local'),('APPDATA','roaming'),('XDG_CONFIG_HOME','config'),('XDG_CACHE_HOME','cache'),('XDG_DATA_HOME','data'),('XDG_STATE_HOME','state'),('TEMP','tmp'),('TMP','tmp')]:
                folder=profile/part;folder.mkdir(exist_ok=True);os.environ[key]=str(folder)
            for key in ('DISPLAY','WAYLAND_DISPLAY','DBUS_SESSION_BUS_ADDRESS'):os.environ.pop(key,None)
            os.environ.update(DBUS_SESSION_BUS_ADDRESS='disabled:',GTK_A11Y='none',NO_AT_BRIDGE='1',GDK_BACKEND='win32',GSK_RENDERER='cairo',STRATA_REQUIRE_GTK_TESTS='1')
            output=msvcrt.get_osfhandle(log.fileno());os.set_handle_inheritable(output,True)
            startup=Startup();startup.cb=c.sizeof(startup);startup.desktop=station.value+'\\'+name;startup.flags=0x100;startup.stdout=output;startup.stderr=output
            if not kernel.CreateProcessW(None,c.create_unicode_buffer(subprocess.list2cmdline(command)),None,None,True,0x08000004,None,None,c.byref(startup),c.byref(process)):raise c.WinError(c.get_last_error())
            if not kernel.AssignProcessToJobObject(job,process.process):raise c.WinError(c.get_last_error())
            if kernel.ResumeThread(process.thread)==0xffffffff:raise c.WinError(c.get_last_error())
            deadline=time.monotonic()+args.timeout;code=w.DWORD(259)
            while time.monotonic()<deadline:
                if not kernel.GetExitCodeProcess(process.process,c.byref(code)):raise c.WinError(c.get_last_error())
                if code.value!=259:break
                time.sleep(.05)
            kernel.CloseHandle(job);job=None
            time.sleep(.05)
            if code.value==259:
                raise RuntimeError(f'Isolated test timed out; log: {args.log}')
            if code.value!=0:
                raise RuntimeError(f'Isolated test exited {code.value}; log: {args.log}')
        text=args.log.read_text(encoding='utf-8',errors='replace')
        counts=re.findall(r'test result: ok\. (\d+) passed;',text)
        if not counts or not all(int(count)>0 for count in counts):
            raise RuntimeError(f'No nonzero passing selection: {args.log}')
        print(f'Private desktop: {counts[-1]} passed; log: {args.log}')
    finally:
        if job:kernel.CloseHandle(job)
        if process.process:
            code=w.DWORD();kernel.GetExitCodeProcess(process.process,c.byref(code))
            if code.value==259:kernel.TerminateProcess(process.process,1)
            kernel.CloseHandle(process.thread);kernel.CloseHandle(process.process)
        user.CloseDesktop(desktop)


if __name__=='__main__':main()
