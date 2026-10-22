// SPDX-License-Identifier: MIT
// Standard Shell interfaces; unsupported browser UI operations remain E_NOTIMPL.

use super::*;

impl IDispatch_Impl for View_Impl {
    fn GetTypeInfoCount(&self) -> windows_core::Result<u32> {
        Ok(0)
    }
    fn GetTypeInfo(&self, _itinfo: u32, _lcid: u32) -> windows_core::Result<ITypeInfo> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn GetIDsOfNames(
        &self,
        _riid: *const windows_core::GUID,
        _rgsznames: *const windows_core::PCWSTR,
        _cnames: u32,
        _lcid: u32,
        _rgdispid: *mut i32,
    ) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn Invoke(
        &self,
        _dispidmember: i32,
        _riid: *const windows_core::GUID,
        _lcid: u32,
        _wflags: DISPATCH_FLAGS,
        _pdispparams: *const DISPPARAMS,
        _pvarresult: *mut windows::Win32::System::Variant::VARIANT,
        _pexcepinfo: *mut EXCEPINFO,
        _puargerr: *mut u32,
    ) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
}

impl IServiceProvider_Impl for View_Impl {
    fn QueryService(
        &self,
        _guidservice: *const windows_core::GUID,
        riid: *const windows_core::GUID,
        ppvobject: *mut *mut core::ffi::c_void,
    ) -> windows_core::Result<()> {
        if riid.is_null() || ppvobject.is_null() {
            return Err(windows_core::Error::from_hresult(E_POINTER));
        }
        // SAFETY: COM supplies the IID and output pointer; QueryInterface initializes the output.
        unsafe {
            self.to_interface::<windows_core::IUnknown>()
                .query(riid, ppvobject)
        }
        .ok()
    }
}

impl IOleWindow_Impl for View_Impl {
    fn GetWindow(&self) -> windows_core::Result<windows::Win32::Foundation::HWND> {
        let window = HWND(self.state.window as *mut _);
        // SAFETY: Querying HWND validity does not access GTK or mutate the window.
        if unsafe { IsWindow(Some(window)) }.as_bool() {
            Ok(window)
        } else {
            Err(windows_core::Error::from_hresult(E_ABORT))
        }
    }
    fn ContextSensitiveHelp(&self, _fentermode: windows_core::BOOL) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
}

impl IShellBrowser_Impl for View_Impl {
    fn InsertMenusSB(
        &self,
        _hmenushared: windows::Win32::UI::WindowsAndMessaging::HMENU,
        _lpmenuwidths: *mut windows::Win32::System::Ole::OLEMENUGROUPWIDTHS,
    ) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn SetMenuSB(
        &self,
        _hmenushared: windows::Win32::UI::WindowsAndMessaging::HMENU,
        _holemenures: isize,
        _hwndactiveobject: windows::Win32::Foundation::HWND,
    ) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn RemoveMenusSB(
        &self,
        _hmenushared: windows::Win32::UI::WindowsAndMessaging::HMENU,
    ) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn SetStatusTextSB(&self, _pszstatustext: &windows_core::PCWSTR) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn EnableModelessSB(&self, _fenable: windows_core::BOOL) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn TranslateAcceleratorSB(
        &self,
        _pmsg: *const windows::Win32::UI::WindowsAndMessaging::MSG,
        _wid: u16,
    ) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn BrowseObject(
        &self,
        _pidl: *const windows::Win32::UI::Shell::Common::ITEMIDLIST,
        _wflags: u32,
    ) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn GetViewStateStream(
        &self,
        _grfmode: u32,
    ) -> windows_core::Result<windows::Win32::System::Com::IStream> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn GetControlWindow(&self, _id: u32) -> windows_core::Result<windows::Win32::Foundation::HWND> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn SendControlMsg(
        &self,
        _id: u32,
        _umsg: u32,
        _wparam: windows::Win32::Foundation::WPARAM,
        _lparam: windows::Win32::Foundation::LPARAM,
        _pret: *mut windows::Win32::Foundation::LRESULT,
    ) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn QueryActiveShellView(&self) -> windows_core::Result<IShellView> {
        Ok(self.to_interface())
    }
    fn OnViewWindowActive(
        &self,
        _pshv: windows_core::Ref<'_, IShellView>,
    ) -> windows_core::Result<()> {
        Ok(())
    }
    fn SetToolbarItems(
        &self,
        _lpbuttons: *const windows::Win32::UI::Controls::TBBUTTON,
        _nbuttons: u32,
        _uflags: u32,
    ) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
}

impl IShellView_Impl for View_Impl {
    fn TranslateAccelerator(
        &self,
        _pmsg: *const windows::Win32::UI::WindowsAndMessaging::MSG,
    ) -> windows_core::HRESULT {
        E_NOTIMPL
    }
    fn EnableModeless(&self, _fenable: windows_core::BOOL) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn UIActivate(&self, ustate: u32) -> windows_core::Result<()> {
        if ustate != 0 {
            let _ = self.state.events.try_send(Event::Show);
        }
        Ok(())
    }
    fn Refresh(&self) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn CreateViewWindow(
        &self,
        _psvprevious: windows_core::Ref<'_, IShellView>,
        _pfs: *const FOLDERSETTINGS,
        _psb: windows_core::Ref<'_, IShellBrowser>,
        _prcview: *const windows::Win32::Foundation::RECT,
    ) -> windows_core::Result<windows::Win32::Foundation::HWND> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn DestroyViewWindow(&self) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn GetCurrentInfo(&self) -> windows_core::Result<FOLDERSETTINGS> {
        Ok(FOLDERSETTINGS {
            ViewMode: FVM_DETAILS.0 as u32,
            fFlags: 0,
        })
    }
    fn AddPropertySheetPages(
        &self,
        _dwreserved: u32,
        _pfn: windows::Win32::UI::Controls::LPFNSVADDPROPSHEETPAGE,
        _lparam: windows::Win32::Foundation::LPARAM,
    ) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn SaveViewState(&self) -> windows_core::Result<()> {
        Ok(())
    }
    fn SelectItem(
        &self,
        pidlitem: *const windows::Win32::UI::Shell::Common::ITEMIDLIST,
        uflags: u32,
    ) -> windows_core::Result<()> {
        if uflags & SVSI_DESELECTOTHERS.0 as u32 != 0 {
            self.state.selected.borrow_mut().clear();
        }
        if pidlitem.is_null() {
            return Ok(());
        }
        let folder = self.state.folder.borrow().clone();
        let path = selected_path(&folder, pidlitem)?;
        let mut selected = self.state.selected.borrow_mut();
        if uflags & SVSI_SELECT.0 as u32 != 0 {
            if !selected.contains(&path) && selected.len() < 256 {
                selected.push(path);
            }
        } else {
            selected.retain(|item| item != &path);
        }
        self.state.selection_pending.set(true);
        Ok(())
    }
    fn GetItemObject(
        &self,
        _uitem: &_SVGIO,
        _riid: *const windows_core::GUID,
        _ppv: *mut *mut core::ffi::c_void,
    ) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
}

impl IWebBrowser_Impl for View_Impl {
    fn GoBack(&self) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn GoForward(&self) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn GoHome(&self) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn GoSearch(&self) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn Navigate(
        &self,
        _url: &windows_core::BSTR,
        _flags: *const windows::Win32::System::Variant::VARIANT,
        _targetframename: *const windows::Win32::System::Variant::VARIANT,
        _postdata: *const windows::Win32::System::Variant::VARIANT,
        _headers: *const windows::Win32::System::Variant::VARIANT,
    ) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn Refresh(&self) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn Refresh2(
        &self,
        _level: *const windows::Win32::System::Variant::VARIANT,
    ) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn Stop(&self) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn Application(&self) -> windows_core::Result<windows::Win32::System::Com::IDispatch> {
        Ok(self.to_interface())
    }
    fn Parent(&self) -> windows_core::Result<windows::Win32::System::Com::IDispatch> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn Container(&self) -> windows_core::Result<windows::Win32::System::Com::IDispatch> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn Document(&self) -> windows_core::Result<windows::Win32::System::Com::IDispatch> {
        Ok(self.to_interface())
    }
    fn TopLevelContainer(&self) -> windows_core::Result<windows::Win32::Foundation::VARIANT_BOOL> {
        Ok(VARIANT_TRUE)
    }
    fn Type(&self) -> windows_core::Result<windows_core::BSTR> {
        Ok(BSTR::from("Strata"))
    }
    fn Left(&self) -> windows_core::Result<i32> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn SetLeft(&self, _left: i32) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn Top(&self) -> windows_core::Result<i32> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn SetTop(&self, _top: i32) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn Width(&self) -> windows_core::Result<i32> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn SetWidth(&self, _width: i32) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn Height(&self) -> windows_core::Result<i32> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn SetHeight(&self, _height: i32) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn LocationName(&self) -> windows_core::Result<windows_core::BSTR> {
        Ok(BSTR::from(
            self.state.folder.borrow().to_string_lossy().as_ref(),
        ))
    }
    fn LocationURL(&self) -> windows_core::Result<windows_core::BSTR> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn Busy(&self) -> windows_core::Result<windows::Win32::Foundation::VARIANT_BOOL> {
        Ok(VARIANT_FALSE)
    }
}

impl IWebBrowserApp_Impl for View_Impl {
    fn Quit(&self) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn ClientToWindow(&self, _pcx: *mut i32, _pcy: *mut i32) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn PutProperty(
        &self,
        _property: &windows_core::BSTR,
        _vtvalue: &windows::Win32::System::Variant::VARIANT,
    ) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn GetProperty(
        &self,
        _property: &windows_core::BSTR,
    ) -> windows_core::Result<windows::Win32::System::Variant::VARIANT> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn Name(&self) -> windows_core::Result<windows_core::BSTR> {
        Ok(BSTR::from("Strata"))
    }
    fn HWND(&self) -> windows_core::Result<windows::Win32::Foundation::SHANDLE_PTR> {
        Ok(SHANDLE_PTR(self.state.window))
    }
    fn FullName(&self) -> windows_core::Result<windows_core::BSTR> {
        Ok(BSTR::from(
            std::env::current_exe()
                .map_err(|_| windows_core::Error::from_hresult(E_FAIL))?
                .to_string_lossy()
                .as_ref(),
        ))
    }
    fn Path(&self) -> windows_core::Result<windows_core::BSTR> {
        Ok(BSTR::from(
            std::env::current_exe()
                .map_err(|_| windows_core::Error::from_hresult(E_FAIL))?
                .parent()
                .ok_or_else(|| windows_core::Error::from_hresult(E_FAIL))?
                .to_string_lossy()
                .as_ref(),
        ))
    }
    fn Visible(&self) -> windows_core::Result<windows::Win32::Foundation::VARIANT_BOOL> {
        // SAFETY: The HWND is borrowed and Windows validates its current visibility.
        Ok(
            if unsafe { IsWindowVisible(HWND(self.state.window as *mut _)) }.as_bool() {
                VARIANT_TRUE
            } else {
                VARIANT_FALSE
            },
        )
    }
    fn SetVisible(
        &self,
        value: windows::Win32::Foundation::VARIANT_BOOL,
    ) -> windows_core::Result<()> {
        if value.0 != 0 {
            let _ = self.state.events.try_send(Event::Show);
        }
        Ok(())
    }
    fn StatusBar(&self) -> windows_core::Result<windows::Win32::Foundation::VARIANT_BOOL> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn SetStatusBar(
        &self,
        _value: windows::Win32::Foundation::VARIANT_BOOL,
    ) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn StatusText(&self) -> windows_core::Result<windows_core::BSTR> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn SetStatusText(&self, _statustext: &windows_core::BSTR) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn ToolBar(&self) -> windows_core::Result<i32> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn SetToolBar(&self, _value: i32) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn MenuBar(&self) -> windows_core::Result<windows::Win32::Foundation::VARIANT_BOOL> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn SetMenuBar(
        &self,
        _value: windows::Win32::Foundation::VARIANT_BOOL,
    ) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn FullScreen(&self) -> windows_core::Result<windows::Win32::Foundation::VARIANT_BOOL> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
    fn SetFullScreen(
        &self,
        _bfullscreen: windows::Win32::Foundation::VARIANT_BOOL,
    ) -> windows_core::Result<()> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
}
