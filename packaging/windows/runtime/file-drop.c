// SPDX-License-Identifier: MIT
// Native OLE format regression on a private window station and clipboard.
#define COBJMACROS
#include <gtk/gtk.h>
#include <windows.h>
#include <ole2.h>
#include <shellapi.h>
#include <shlobj.h>

static const wchar_t *paths[] = {
  L"C:\\Strata fixture\\\u1ea3nh # 100%.png",
  L"\\\\server\\shared folder\\\u65e5\u672c\u8a9e.txt"
};
static gboolean baseline;

static DWORD WINAPI
read_files (void *unused)
{
  (void) unused;
  HRESULT hr = OleInitialize (NULL);
  if (FAILED (hr)) return 2;
  FORMATETC format = { CF_HDROP, NULL, DVASPECT_CONTENT, -1, TYMED_HGLOBAL };
  FORMATETC registered = format;
  registered.cfFormat = RegisterClipboardFormatW (L"CF_HDROP");
  IDataObject *object = NULL;
  DWORD deadline = GetTickCount () + 10000;
  do {
    if (object) IDataObject_Release (object);
    object = NULL;
    hr = OleGetClipboard (&object);
    if (SUCCEEDED (hr) &&
        (SUCCEEDED (IDataObject_QueryGetData (object, &format)) ||
         SUCCEEDED (IDataObject_QueryGetData (object, &registered)))) break;
    Sleep (10);
  } while ((LONG) (deadline - GetTickCount ()) > 0);
  LONG status = 3;
  if (object) {
    if (baseline) {
      status = FAILED (IDataObject_QueryGetData (object, &format)) &&
               SUCCEEDED (IDataObject_QueryGetData (object, &registered)) ? 0 : 4;
    } else {
      STGMEDIUM medium = { 0 };
      hr = IDataObject_GetData (object, &format, &medium);
      if (SUCCEEDED (hr)) {
        status = DragQueryFileW ((HDROP) medium.hGlobal, 0xffffffff, NULL, 0) == G_N_ELEMENTS (paths) ? 0 : 5;
        for (UINT i = 0; status == 0 && i < G_N_ELEMENTS (paths); i++) {
          UINT length = DragQueryFileW ((HDROP) medium.hGlobal, i, NULL, 0);
          wchar_t *name = g_new0 (wchar_t, length + 1);
          DragQueryFileW ((HDROP) medium.hGlobal, i, name, length + 1);
          if (wcscmp (name, paths[i]) != 0) status = 6;
          g_free (name);
        }
        ReleaseStgMedium (&medium);
      }
    }
    IDataObject_Release (object);
  }
  OleUninitialize ();
  return (DWORD) status;
}

int
main (int argc, char **argv)
{
  baseline = argc == 2 && strcmp (argv[1], "--expect-unsupported") == 0;
  wchar_t station_name[64];
  swprintf (station_name, G_N_ELEMENTS (station_name), L"StrataFileDrop-%lu", GetCurrentProcessId ());
  HWINSTA station = CreateWindowStationW (station_name, CWF_CREATE_ONLY, WINSTA_ALL_ACCESS, NULL);
  /* Unprivileged callers cannot choose a name. Both paths require creation:
   * never open an existing station or reuse the user's clipboard. */
  if (!station && GetLastError () == ERROR_ACCESS_DENIED)
    station = CreateWindowStationW (NULL, CWF_CREATE_ONLY, WINSTA_ALL_ACCESS, NULL);
  if (!station) {
    g_printerr ("CreateWindowStation failed: %lu\n", GetLastError ());
    return 10;
  }
  if (!SetProcessWindowStation (station)) {
    g_printerr ("SetProcessWindowStation failed: %lu\n", GetLastError ());
    return 10;
  }
  HDESK desktop = CreateDesktopW (L"StrataFileDrop", NULL, NULL, 0, GENERIC_ALL, NULL);
  if (!desktop || !SetThreadDesktop (desktop)) return 11;
  gtk_init ();
  wchar_t module[MAX_PATH];
  GetModuleFileNameW (GetModuleHandleW (L"libgtk-4-1.dll"), module, G_N_ELEMENTS (module));
  char *module_utf8 = g_utf16_to_utf8 ((gunichar2 *) module, -1, NULL, NULL, NULL);
  g_print ("GTK runtime: %s\n", module_utf8);
  g_free (module_utf8);
  DROPFILES header = { .pFiles = sizeof (DROPFILES), .fWide = TRUE };
  GByteArray *data = g_byte_array_new ();
  g_byte_array_append (data, (const guint8 *) &header, sizeof (header));
  for (size_t i = 0; i < G_N_ELEMENTS (paths); i++)
    g_byte_array_append (data, (const guint8 *) paths[i], (wcslen (paths[i]) + 1) * sizeof (wchar_t));
  const wchar_t end = 0;
  g_byte_array_append (data, (const guint8 *) &end, sizeof (end));
  GBytes *bytes = g_byte_array_free_to_bytes (data);
  GdkContentProvider *provider = gdk_content_provider_new_for_bytes ("application/x.windows.CF_HDROP", bytes);
  if (!gdk_clipboard_set_content (gdk_display_get_clipboard (gdk_display_get_default ()), provider)) return 12;
  HANDLE reader = CreateThread (NULL, 0, read_files, NULL, 0, NULL);
  if (!reader) return 13;
  ULONGLONG deadline = GetTickCount64 () + 15000;
  while (WaitForSingleObject (reader, 0) != WAIT_OBJECT_0) {
    if (GetTickCount64 () > deadline) return 14;
    while (g_main_context_pending (NULL)) g_main_context_iteration (NULL, FALSE);
    Sleep (1);
  }
  DWORD exit_code;
  GetExitCodeThread (reader, &exit_code);
  CloseHandle (reader);
  g_print ("Native file formats: %s (%lu)\n", baseline ? "baseline unsupported" : "CF_HDROP round-trip", exit_code);
  g_object_unref (provider);
  g_bytes_unref (bytes);
  return (int) exit_code;
}
