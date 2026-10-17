"""Denoise raw linear Cycles irradiance with OIDN's dedicated RTLightmap filter."""
import ctypes as c
import numpy as np


def check_error(library, device):
    message = c.c_char_p()
    code = library.oidnGetDeviceError(device, c.byref(message))
    if code:
        raise RuntimeError(f'OIDN {code}: {message.value.decode() if message.value else "unknown error"}')


def denoise(pixels, library_path):
    library = c.CDLL(str(library_path))
    signatures = {
        'oidnNewDevice': ([c.c_int], c.c_void_p),
        'oidnCommitDevice': ([c.c_void_p], None),
        'oidnNewFilter': ([c.c_void_p,c.c_char_p], c.c_void_p),
        'oidnSetSharedFilterImage': ([c.c_void_p,c.c_char_p,c.c_void_p,c.c_int]+[c.c_size_t]*5, None),
        'oidnCommitFilter': ([c.c_void_p], None),
        'oidnExecuteFilter': ([c.c_void_p], None),
        'oidnGetDeviceError': ([c.c_void_p,c.POINTER(c.c_char_p)], c.c_int),
        'oidnReleaseFilter': ([c.c_void_p], None),
        'oidnReleaseDevice': ([c.c_void_p], None),
    }
    for name,(arguments,result) in signatures.items():
        function = getattr(library,name)
        function.argtypes, function.restype = arguments,result
    device, filter_handle = library.oidnNewDevice(1), None  # CPU
    try:
        check_error(library,device)
        if not device: raise RuntimeError('OIDN did not create a CPU device')
        library.oidnCommitDevice(device)
        check_error(library,device)
        filter_handle = library.oidnNewFilter(device,b'RTLightmap')
        check_error(library,device)
        if not filter_handle: raise RuntimeError('OIDN did not create RTLightmap')
        source = np.ascontiguousarray(pixels,dtype=np.float32)
        if source.ndim != 3 or source.shape[2] != 3 or not np.isfinite(source).all():
            raise ValueError('Expected finite RGB irradiance')
        output = np.empty_like(source)
        height,width,_ = source.shape
        for name,array in [(b'color',source),(b'output',output)]:
            library.oidnSetSharedFilterImage(filter_handle,name,array.ctypes.data,3,width,height,0,0,0)
        library.oidnCommitFilter(filter_handle)
        check_error(library,device)
        library.oidnExecuteFilter(filter_handle)
        check_error(library,device)
        if not np.isfinite(output).all(): raise RuntimeError('Non-finite denoised irradiance')
        return np.maximum(output,0)
    finally:
        if filter_handle: library.oidnReleaseFilter(filter_handle)
        if device: library.oidnReleaseDevice(device)
