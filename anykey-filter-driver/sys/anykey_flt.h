/*++
  AnyKey Filter Driver -- anykey_flt.h
  Internal driver header: device extension, function declarations.
  v0.2: mouse support added alongside keyboard filtering.
--*/

#ifndef _ANYKEY_FLT_H
#define _ANYKEY_FLT_H

#include <ntddk.h>
#include <wdf.h>
#include <ntddkbd.h>
#include <ntddmou.h>
#include <kbdmou.h>
#include <initguid.h>
#include <devguid.h>
#include <ntstrsafe.h>

#include "public.h"

// -- Pool tag for all allocations --
#define ANYKEY_POOL_TAG 'KyAn'   // 'AnyK' little-endian

// -- Queue depth --
#define ANYKEY_MAX_INPUT_QUEUE  64

// -- Device extension for each filter device (one per keyboard/mouse) --
typedef struct _DEVICE_EXTENSION {

    // -- From kbfiltr: connection data --
    CONNECT_DATA        UpperConnectData;   // Original kbdclass callback+device
    KEYBOARD_ATTRIBUTES KeyboardAttributes;

    // -- WDF queue handles --
    WDFQUEUE            RawPdoQueue;        // Queue for forwarded IOCTLs from RawPDO

    // -- Device type (v0.2) --
    UINT8               DeviceType;         // ANYKEY_DEV_KEYBOARD or ANYKEY_DEV_MOUSE

    // -- Device identity --
    ULONG               DeviceId;           // Monotonic index (starts at 1)
    WCHAR               HardwareId[128];    // e.g., "HID\VID_046D&PID_C539&..."
    WCHAR               ContainerId[40];    // ContainerID GUID string
    WCHAR               FriendlyName[64];   // PnP device description
    USHORT              VendorId;           // Numeric VID
    USHORT              ProductId;          // Numeric PID
    BOOLEAN             HasSerialNumber;    // USB iSerialNumber != 0
    ULONG               DeviceFlags;        // ANYKEY_DEV_FLAG_*

    // -- Keyboard input queue (protected by InputQueueLock, at DISPATCH_LEVEL) --
    KSPIN_LOCK          InputQueueLock;
    ANYKEY_INPUT_EVENT  InputQueue[ANYKEY_MAX_INPUT_QUEUE];
    ULONG               InputQueueHead;     // Next read position
    ULONG               InputQueueTail;     // Next write position
    ULONG               InputQueueCount;    // Number of pending events

    // -- Mouse support (v0.2) --
    CONNECT_DATA        MouseConnectData;   // Original mouclass callback+device
    KSPIN_LOCK          MouseQueueLock;
    ANYKEY_MOUSE_EVENT  MouseQueue[ANYKEY_MAX_INPUT_QUEUE];
    ULONG               MouseQueueHead;     // Next read position
    ULONG               MouseQueueTail;     // Next write position
    ULONG               MouseQueueCount;    // Number of pending events

    // -- List link for global device list --
    LIST_ENTRY          ListEntry;          // Embedded in global device list

    // -- Per-device intercept (v0.4) --
    // Default FALSE (hotplug safe). Engine sets via IOCTL_SET_INTERCEPT.
    BOOLEAN             InterceptEnabled;   // TRUE = intercept this device's input

} DEVICE_EXTENSION, *PDEVICE_EXTENSION;

WDF_DECLARE_CONTEXT_TYPE_WITH_NAME(DEVICE_EXTENSION, FilterGetData)

// -- RawPDO device extension --
typedef struct _RPDO_DEVICE_DATA {
    ULONG       InstanceNo;
    WDFQUEUE    ParentQueue;    // Queue on the parent filter device
} RPDO_DEVICE_DATA, *PRPDO_DEVICE_DATA;

WDF_DECLARE_CONTEXT_TYPE_WITH_NAME(RPDO_DEVICE_DATA, PdoGetData)

// -- Function declarations --

DRIVER_INITIALIZE DriverEntry;

// Unified EvtDeviceAdd -- handles both keyboard and mouse filter devices
EVT_WDF_DRIVER_DEVICE_ADD       AnyKey_EvtDeviceAdd;

// Keyboard ServiceCallback (hooked into kbdclass callback chain)
// WDK 10.0.28000+ uses opaque PVOID parameters (kbdmou.h PSERVICE_CALLBACK_ROUTINE)
VOID KbFilter_ServiceCallback(
    _In_     PVOID    NormalContext,
    _In_     PVOID    SystemArgument1,
    _In_     PVOID    SystemArgument2,
    _Inout_  PVOID    SystemArgument3
);

// Mouse ServiceCallback (v0.2 -- hooked into mouclass callback chain)
VOID MouFilter_ServiceCallback(
    _In_     PVOID    NormalContext,
    _In_     PVOID    SystemArgument1,
    _In_     PVOID    SystemArgument2,
    _Inout_  PVOID    SystemArgument3
);

// IOCTL handlers for user-mode communication (control device + RawPDO)
EVT_WDF_IO_QUEUE_IO_DEVICE_CONTROL
                                 KbFilter_EvtIoDeviceControlFromRawPdo;

// RawPDO creation
NTSTATUS KbFiltr_CreateRawPdo(
    _In_ WDFDEVICE               Device,
    _In_ ULONG                   InstanceNo
);

// Debug print helper -- always enabled for driver testing
#define AnyKeyDebugPrint(x, ...) DbgPrintEx(DPFLTR_IHVDRIVER_ID, DPFLTR_ERROR_LEVEL, \
    "[AnyKeyFlt] " x, __VA_ARGS__)

#endif // _ANYKEY_FLT_H
