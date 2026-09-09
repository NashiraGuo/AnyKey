/*++
  AnyKey Filter Driver — anykey_flt.c
  Keyboard + Mouse class filter driver: intercept input, inject output.
  Installed as UpperFilter of keyboard and mouse classes.
  v0.3: emergency combo (LCtrl+Space+Esc) + fixed E0-prefixed modifier release.

  === SAFETY ===
  1. Emergency combo: LCtrl+Space+Esc -> immediate shutdown (releases all held
     modifiers/mouse buttons, disables interception, flushes queues).
     Checked at highest priority BEFORE heartbeat watchdog.
  2. Watchdog: if engine doesn't poll within 30s, interception auto-disables.
  3. Safe Mode: UpperFilters are NOT loaded in Safe Mode.
  4. Pass-through default: InterceptEnabled starts FALSE.
  5. sc stop anykey_flt / sc delete anykey_flt removes the filter.
--*/

#include "anykey_flt.h"

// ===============================================================
// Global state shared across all filter device instances
// ===============================================================

typedef struct _ANYKEY_GLOBAL {
    KSPIN_LOCK          ListLock;
    LIST_ENTRY          DeviceListHead;     // List of DEVICE_EXTENSION.ListEntry
    LONG                NextDeviceId;       // Starts at 1

    // ─ Session + Health ─
    BOOLEAN             CaptureEnabled;     // Passive passthrough capture for GUI identify (mirrors input, no consume)
    BOOLEAN             SessionActive;      // TRUE when engine has opened handle

    // ── Heartbeat (Session+Heartbeat safety model) ──
    // Updated on IOCTL_ANYKEY_HEARTBEAT by dedicated watchdog thread.
    // Uses KeQueryInterruptTime (not system time) for monotonic comparison.
    LARGE_INTEGER       LastHeartbeat;      // KeQueryInterruptTime at last heartbeat

    // Event-driven input notification (IOCTL_ANYKEY_SET_EVENT)
    // Shared by both KbFilter and MouFilter ServiceCallbacks.
    PKEVENT             hInputEvent;        // Valid KEVENT ptr after user-mode register
    PVOID               hInputEventObject;  // ObDereferenceObject on cleanup

    // ── Device list change tracking ──
    BOOLEAN             DeviceListChanged;   // TRUE when devices added/removed

    // ── Mouse gesture mode (v0.2) ──
    BOOLEAN             MouseGesturing;      // TRUE = queue+signal movement, FALSE = forward-only

    // ── Emergency combo state (LCtrl+Space+Esc -> emergency shutdown) ──
    // Updated from KbFilter_ServiceCallback at DISPATCH_LEVEL.
    // Single BOOLEAN reads/writes are atomic on x86/x64 — no spinlock needed.
    BOOLEAN             Emergency_Lctrl;     // TRUE while Left Ctrl is held
    BOOLEAN             Emergency_Space;     // TRUE while Space is held
    BOOLEAN             Emergency_Esc;       // TRUE while Escape is held
} ANYKEY_GLOBAL;

// Safety timeout: if engine doesn't heartbeat for this many seconds, emergency shutdown
#define ANYKEY_HEARTBEAT_TIMEOUT_SECONDS  30

static ANYKEY_GLOBAL g_AnyKey = { 0 };

// Forward declarations
static VOID AnyKey_EvtFileCleanup(_In_ WDFFILEOBJECT FileObject);
static VOID AnyKey_EmergencyShutdown(VOID);
static VOID KbFilter_EvtIoInternalDeviceControl(
    _In_ WDFQUEUE Queue, _In_ WDFREQUEST Request,
    _In_ size_t OutputBufferLength, _In_ size_t InputBufferLength,
    _In_ ULONG IoControlCode);
static VOID MouFilter_EvtIoInternalDeviceControl(
    _In_ WDFQUEUE Queue, _In_ WDFREQUEST Request,
    _In_ size_t OutputBufferLength, _In_ size_t InputBufferLength,
    _In_ ULONG IoControlCode);
static NTSTATUS AnyKey_DrainMouseQueues(
    _Out_writes_bytes_to_(OutputBufferLength, *BytesReturned)
         PVOID OutputBuffer, _In_ size_t OutputBufferLength, _Out_ size_t* BytesReturned);
static NTSTATUS AnyKey_InjectMouseOutput(_In_ PANYKEY_MOUSE_OUTPUT_EVENT OutputEvent);
static UINT8 AnyKey_DetectDeviceType(_In_ PDEVICE_OBJECT PdoObject);

// ===============================================================
// DriverEntry
// ===============================================================

NTSTATUS
DriverEntry(
    _In_ PDRIVER_OBJECT  DriverObject,
    _In_ PUNICODE_STRING RegistryPath
)
{
    WDF_DRIVER_CONFIG config;
    NTSTATUS status;

    AnyKeyDebugPrint("AnyKey Filter Driver v0.2.0 (kb+mouse)\n");

    // Init global state
    KeInitializeSpinLock(&g_AnyKey.ListLock);
    InitializeListHead(&g_AnyKey.DeviceListHead);
    g_AnyKey.NextDeviceId = 1;
    g_AnyKey.CaptureEnabled = FALSE;
    g_AnyKey.SessionActive = FALSE;
    g_AnyKey.LastHeartbeat.QuadPart = 0;

    WDF_DRIVER_CONFIG_INIT(&config, AnyKey_EvtDeviceAdd);

    status = WdfDriverCreate(
        DriverObject,
        RegistryPath,
        WDF_NO_OBJECT_ATTRIBUTES,
        &config,
        WDF_NO_HANDLE);

    if (!NT_SUCCESS(status)) {
        AnyKeyDebugPrint("WdfDriverCreate failed: 0x%x\n", status);
        return status;
    }

    // ── Create a control device for user-mode IOCTL access ──
    // Control devices are NOT in the PnP device stack, don't go through
    // START_DEVICE (no STATUS_INVALID_DEVICE_STATE like RawPDO), and can
    // have a custom SDDL (unlike filter FDOs which inherit stack security).
    // This is the WDF-standard pattern for a user-accessible side channel.
    {
        DECLARE_CONST_UNICODE_STRING(ctrlSDDL, L"D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;IU)");
        DECLARE_CONST_UNICODE_STRING(ctrlName, L"\\Device\\AnyKeyFlt");

        PWDFDEVICE_INIT pCtrlInit = WdfControlDeviceInitAllocate(
            WdfGetDriver(), &ctrlSDDL);
        if (pCtrlInit == NULL) {
            AnyKeyDebugPrint("WdfControlDeviceInitAllocate failed\n");
            return STATUS_INSUFFICIENT_RESOURCES;
        }

        WdfDeviceInitSetDeviceType(pCtrlInit, FILE_DEVICE_UNKNOWN);
        WdfDeviceInitAssignName(pCtrlInit, &ctrlName);

        WDF_OBJECT_ATTRIBUTES ctrlAttr;
        WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&ctrlAttr, DEVICE_EXTENSION);

        // FileCleanup: when the last handle closes (process exit / kill),
        // auto-disable interception so keyboard isn't stuck.
        WDF_FILEOBJECT_CONFIG fileConfig;
        WDF_FILEOBJECT_CONFIG_INIT(&fileConfig, NULL, NULL, AnyKey_EvtFileCleanup);
        WdfDeviceInitSetFileObjectConfig(pCtrlInit, &fileConfig, WDF_NO_OBJECT_ATTRIBUTES);

        WDFDEVICE hControl;
        status = WdfDeviceCreate(&pCtrlInit, &ctrlAttr, &hControl);
        if (!NT_SUCCESS(status)) {
            AnyKeyDebugPrint("Control WdfDeviceCreate failed: 0x%x\n", status);
            WdfDeviceInitFree(pCtrlInit);
            return status;
        }

        // Queue for user-mode IOCTLs — reuses the existing handler which
        // operates on global g_AnyKey state (not per-device extension).
        WDF_IO_QUEUE_CONFIG ctrlQ;
        WDF_IO_QUEUE_CONFIG_INIT_DEFAULT_QUEUE(&ctrlQ, WdfIoQueueDispatchSequential);
        ctrlQ.EvtIoDeviceControl = KbFilter_EvtIoDeviceControlFromRawPdo;

        WDFQUEUE hCtrlQueue;
        status = WdfIoQueueCreate(hControl, &ctrlQ,
                                   WDF_NO_OBJECT_ATTRIBUTES, &hCtrlQueue);
        if (!NT_SUCCESS(status)) {
            AnyKeyDebugPrint("Control WdfIoQueueCreate failed: 0x%x\n", status);
            return status;
        }

        // Control devices are non-PnP — WdfDeviceCreateDeviceInterface
        // returns STATUS_INVALID_DEVICE_REQUEST. Use a symbolic link instead.
        DECLARE_CONST_UNICODE_STRING(symLink, L"\\DosDevices\\AnyKeyFlt");
        status = WdfDeviceCreateSymbolicLink(hControl, &symLink);
        if (!NT_SUCCESS(status)) {
            AnyKeyDebugPrint("Control WdfDeviceCreateSymbolicLink failed: 0x%x\n", status);
            return status;
        }

        WdfControlFinishInitializing(hControl);
        AnyKeyDebugPrint("Control device created (user-mode IOCTL channel)\n");
    }

    return status;
}

// ===============================================================
// EvtDeviceContextCleanup — remove device from global list
// ===============================================================

static VOID
AnyKey_EvtDeviceContextCleanup(
    _In_ WDFOBJECT Object
)
{
    WDFDEVICE           hDevice = (WDFDEVICE)Object;
    PDEVICE_EXTENSION   devExt = FilterGetData(hDevice);
    KIRQL               oldIrql;

    AnyKeyDebugPrint("Device %lu cleanup: removing from global list\n", devExt->DeviceId);

    KeAcquireSpinLock(&g_AnyKey.ListLock, &oldIrql);
    RemoveEntryList(&devExt->ListEntry);
    KeReleaseSpinLock(&g_AnyKey.ListLock, oldIrql);

    // Notify engine: device list changed
    g_AnyKey.DeviceListChanged = TRUE;
    if (g_AnyKey.hInputEvent) {
        KeSetEvent(g_AnyKey.hInputEvent, IO_NO_INCREMENT, FALSE);
    }
}

// ===============================================================
// EvtFileCleanup — called when the last handle to the control device
// is closed (process exit / forced termination). Auto-disables
// interception so keyboard isn't left in a dead state.
// ===============================================================

static VOID
AnyKey_EvtFileCleanup(
    _In_ WDFFILEOBJECT FileObject
)
{
    UNREFERENCED_PARAMETER(FileObject);

    // v0.4: per-device intercept — disable all devices on session close
    KIRQL oldIrql;
    PLIST_ENTRY entry;
    KeAcquireSpinLock(&g_AnyKey.ListLock, &oldIrql);
    for (entry = g_AnyKey.DeviceListHead.Flink;
         entry != &g_AnyKey.DeviceListHead;
         entry = entry->Flink) {
        PDEVICE_EXTENSION devExt = CONTAINING_RECORD(entry, DEVICE_EXTENSION, ListEntry);
        devExt->InterceptEnabled = FALSE;
    }
    KeReleaseSpinLock(&g_AnyKey.ListLock, oldIrql);

    g_AnyKey.SessionActive = FALSE;
    AnyKeyDebugPrint("FileCleanup: session closed, all devices set to passthrough\n");
}

// ===============================================================
// EvtDeviceAdd — called for each keyboard AND mouse device in the class
// v0.2: unified handler for both device types.
// ===============================================================

NTSTATUS
AnyKey_EvtDeviceAdd(
    _In_ WDFDRIVER        Driver,
    _Inout_ PWDFDEVICE_INIT DeviceInit
)
{
    WDF_OBJECT_ATTRIBUTES   deviceAttributes;
    WDFDEVICE               hDevice;
    PDEVICE_EXTENSION       devExt;
    WDF_IO_QUEUE_CONFIG     ioQueueConfig;
    WDFQUEUE                hQueue;
    NTSTATUS                status;
    UINT8                   devType = ANYKEY_DEV_KEYBOARD;

    UNREFERENCED_PARAMETER(Driver);

    AnyKeyDebugPrint("AnyKey_EvtDeviceAdd enter\n");

    // Set as filter driver
    WdfFdoInitSetFilter(DeviceInit);

    // Allocate device extension + register cleanup callback
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&deviceAttributes, DEVICE_EXTENSION);
    deviceAttributes.EvtCleanupCallback = AnyKey_EvtDeviceContextCleanup;

    status = WdfDeviceCreate(&DeviceInit, &deviceAttributes, &hDevice);
    if (!NT_SUCCESS(status)) {
        AnyKeyDebugPrint("WdfDeviceCreate failed: 0x%x\n", status);
        return status;
    }

    devExt = FilterGetData(hDevice);
    RtlZeroMemory(devExt, sizeof(DEVICE_EXTENSION));

    // Initialize device extension common fields
    KeInitializeSpinLock(&devExt->InputQueueLock);
    devExt->InputQueueHead = 0;
    devExt->InputQueueTail = 0;
    devExt->InputQueueCount = 0;
    devExt->HardwareId[0] = L'\0';
    devExt->ContainerId[0] = L'\0';
    devExt->FriendlyName[0] = L'\0';
    devExt->VendorId = 0;
    devExt->ProductId = 0;
    devExt->HasSerialNumber = FALSE;
    devExt->DeviceFlags = 0;
    devExt->DeviceType = ANYKEY_DEV_KEYBOARD;  // default
    devExt->InterceptEnabled = FALSE;           // v0.4: default passthrough (hotplug safe)

    // Assign device ID (starts at 1; 0 is reserved as wildcard)
    devExt->DeviceId = (ULONG)InterlockedIncrement(&g_AnyKey.NextDeviceId);

    // Add to global device list
    {
        KIRQL oldIrql;
        KeAcquireSpinLock(&g_AnyKey.ListLock, &oldIrql);
        InsertTailList(&g_AnyKey.DeviceListHead, &devExt->ListEntry);
        KeReleaseSpinLock(&g_AnyKey.ListLock, oldIrql);
    }

    AnyKeyDebugPrint("Device %lu added to global list\n", devExt->DeviceId);

    // ── Query PnP device identity + detect device type ──
    {
        PDEVICE_OBJECT pdoObj = WdfDeviceWdmGetPhysicalDevice(hDevice);
        if (pdoObj) {
            // Detect device type (keyboard vs mouse) via Class GUID
            devType = AnyKey_DetectDeviceType(pdoObj);
            devExt->DeviceType = devType;
            AnyKeyDebugPrint("Device %lu type=%s\n", devExt->DeviceId,
                             devType == ANYKEY_DEV_MOUSE ? "MOUSE" : "KEYBOARD");

            // 1. HardwareId (multi-sz; keep only the FIRST/most-specific string)
            //    IoGetDeviceProperty is all-or-nothing: the buffer must hold the
            //    ENTIRE multi-sz list, else it returns STATUS_BUFFER_TOO_SMALL and
            //    writes nothing. The old 128-WCHAR (256B) buffer silently failed on
            //    composite HID devices whose HardwareId list exceeds 256B (e.g. ROG
            //    keyboard ~316B), leaving HardwareId/VID/PID empty. Enlarged to
            //    512 WCHAR (1024B). RtlStringCbCopyW stops at the first NUL, so it
            //    naturally copies just the first (most specific) HardwareId string.
            {
                WCHAR buf[512] = { 0 };
                ULONG len = 0;
                NTSTATUS s = IoGetDeviceProperty(pdoObj,
                    DevicePropertyHardwareID, sizeof(buf), buf, &len);
                if (NT_SUCCESS(s) && len > 0) {
                    // Guarantee NUL-termination even if the list exactly filled buf.
                    buf[(sizeof(buf) / sizeof(WCHAR)) - 1] = L'\0';
                    RtlStringCbCopyW(devExt->HardwareId, sizeof(devExt->HardwareId), buf);
                    // Parse VID/PID: "HID\VID_046D&PID_C539&..."
                    // Manual hex parse (kernel mode — no wcstoul)
                    PCWSTR p = wcsstr(buf, L"VID_");
                    if (p) {
                        USHORT v = 0;
                        for (int i = 4; i < 8; i++) {
                            WCHAR c = p[i];
                            v <<= 4;
                            if (c >= L'0' && c <= L'9') v |= (USHORT)(c - L'0');
                            else if (c >= L'A' && c <= L'F') v |= (USHORT)(c - L'A' + 10);
                            else if (c >= L'a' && c <= L'f') v |= (USHORT)(c - L'a' + 10);
                            else break;
                        }
                        devExt->VendorId = v;
                    }
                    p = wcsstr(buf, L"PID_");
                    if (p) {
                        USHORT v = 0;
                        for (int i = 4; i < 8; i++) {
                            WCHAR c = p[i];
                            v <<= 4;
                            if (c >= L'0' && c <= L'9') v |= (USHORT)(c - L'0');
                            else if (c >= L'A' && c <= L'F') v |= (USHORT)(c - L'A' + 10);
                            else if (c >= L'a' && c <= L'f') v |= (USHORT)(c - L'a' + 10);
                            else break;
                        }
                        devExt->ProductId = v;
                    }
                    AnyKeyDebugPrint("Device %lu hwid=%S vid=%04X pid=%04X (len=%lu)\n",
                                     devExt->DeviceId, devExt->HardwareId,
                                     devExt->VendorId, devExt->ProductId, len);
                } else {
                    AnyKeyDebugPrint("Device %lu HardwareId query failed status=0x%x len=%lu\n",
                                     devExt->DeviceId, s, len);
                }
            }
            // 2. ContainerID
            {
                WCHAR buf[40] = { 0 };
                ULONG len = 0;
                NTSTATUS s = IoGetDeviceProperty(pdoObj,
                    DevicePropertyContainerID, sizeof(buf), buf, &len);
                if (NT_SUCCESS(s) && len > 0) {
                    RtlStringCbCopyW(devExt->ContainerId, sizeof(devExt->ContainerId), buf);
                }
            }
            // 3. FriendlyName
            {
                WCHAR buf[64] = { 0 };
                ULONG len = 0;
                NTSTATUS s = IoGetDeviceProperty(pdoObj,
                    DevicePropertyFriendlyName, sizeof(buf), buf, &len);
                if (NT_SUCCESS(s) && len > 0) {
                    RtlStringCbCopyW(devExt->FriendlyName, sizeof(devExt->FriendlyName), buf);
                } else {
                    // Fallback: use device description
                    s = IoGetDeviceProperty(pdoObj,
                        DevicePropertyDeviceDescription, sizeof(buf), buf, &len);
                    if (NT_SUCCESS(s) && len > 0) {
                        RtlStringCbCopyW(devExt->FriendlyName, sizeof(devExt->FriendlyName), buf);
                    }
                }
            }
            // 4. USB serial number (via bus query; default FALSE)
            devExt->HasSerialNumber = FALSE;
            // 5. Device flags
            devExt->DeviceFlags = 0;
            if (devType == ANYKEY_DEV_MOUSE) {
                devExt->DeviceFlags |= ANYKEY_DEV_FLAG_MOUSE;
            }
        }
    }

    // Initialize mouse queue fields (zeroed by RtlZeroMemory, but explicit for clarity)
    devExt->MouseConnectData.ClassService = NULL;
    KeInitializeSpinLock(&devExt->MouseQueueLock);
    devExt->MouseQueueHead = 0;
    devExt->MouseQueueTail = 0;
    devExt->MouseQueueCount = 0;

    // Default queue — handles internal IOCTLs (CONNECT/DISCONNECT etc).
    // Route mouse vs keyboard via device type.
    WDF_IO_QUEUE_CONFIG_INIT_DEFAULT_QUEUE(&ioQueueConfig, WdfIoQueueDispatchSequential);

    if (devType == ANYKEY_DEV_MOUSE) {
        ioQueueConfig.EvtIoInternalDeviceControl = MouFilter_EvtIoInternalDeviceControl;
    } else {
        ioQueueConfig.EvtIoInternalDeviceControl = KbFilter_EvtIoInternalDeviceControl;
    }

    status = WdfIoQueueCreate(hDevice, &ioQueueConfig, WDF_NO_OBJECT_ATTRIBUTES, &hQueue);
    if (!NT_SUCCESS(status)) {
        AnyKeyDebugPrint("WdfIoQueueCreate(default) failed: 0x%x\n", status);
        return status;
    }

    // Create queue for RawPDO forwarded IOCTLs
    WDF_IO_QUEUE_CONFIG_INIT(&ioQueueConfig, WdfIoQueueDispatchParallel);
    ioQueueConfig.EvtIoDeviceControl = KbFilter_EvtIoDeviceControlFromRawPdo;

    status = WdfIoQueueCreate(hDevice, &ioQueueConfig, WDF_NO_OBJECT_ATTRIBUTES, &hQueue);
    if (!NT_SUCCESS(status)) {
        AnyKeyDebugPrint("WdfIoQueueCreate(rawPdoQueue) failed: 0x%x\n", status);
        return status;
    }
    devExt->RawPdoQueue = hQueue;

    AnyKeyDebugPrint("AnyKey_EvtDeviceAdd success, DeviceId=%lu type=%s\n",
                     devExt->DeviceId,
                     devType == ANYKEY_DEV_MOUSE ? "MOUSE" : "KEYBOARD");
    return STATUS_SUCCESS;
}

// ===============================================================
// AnyKey_DetectDeviceType — determine if PDO is keyboard or mouse
// Uses Class GUID: {4D36E96B} = Keyboard, {4D36E96F} = Mouse.
// ===============================================================

static UINT8
AnyKey_DetectDeviceType(
    _In_ PDEVICE_OBJECT PdoObject
)
{
    WCHAR classGuidStr[80] = { 0 };
    ULONG len = 0;

    // DevicePropertyClassGuid returns a GUID string
    // Keyboard: {4D36E96B-E325-11CE-BFC1-08002BE10318}
    // Mouse:    {4D36E96F-E325-11CE-BFC1-08002BE10318}
    NTSTATUS status = IoGetDeviceProperty(
        PdoObject,
        DevicePropertyClassGuid,
        sizeof(classGuidStr),
        classGuidStr,
        &len);

    if (NT_SUCCESS(status) && len > 0) {
        // The GUID string includes curly braces; check the 8 hex chars after "{"
        // Keyboard: "{4d36e96b-..."
        // Mouse:    "{4d36e96f-..."
        if (classGuidStr[0] == L'{') {
            // Compare last hex digit of the first group
            // Keyboard = 0x000B (decimal 11) -> "...6b-"
            // Mouse    = 0x000F (decimal 15) -> "...6f-"
            PCWSTR p = wcschr(classGuidStr, L'-');
            if (p && p > classGuidStr) {
                // p points to the last char before '-', check if it's 'f' (mouse)
                if (*(p - 1) == L'f' || *(p - 1) == L'F') {
                    AnyKeyDebugPrint("DetectDeviceType: MOUSE (GUID=%S)\n", classGuidStr);
                    return ANYKEY_DEV_MOUSE;
                }
            }
        }
    }

    // Default to keyboard
    AnyKeyDebugPrint("DetectDeviceType: KEYBOARD (GUID=%S, status=0x%x)\n",
                     classGuidStr, status);
    return ANYKEY_DEV_KEYBOARD;
}

// ===============================================================
// EvtIoInternalDeviceControl -- WDF queue callback (kbfiltr pattern).
// Intercepts CONNECT/DISCONNECT, forwards EVERYTHING to the port driver
// via WdfRequestSend. This is the proven mechanism: the port driver
// receives the modified CONNECT data and registers our ServiceCallback.
// ===============================================================

static VOID
KbFilter_EvtIoInternalDeviceControl(
    _In_ WDFQUEUE      Queue,
    _In_ WDFREQUEST    Request,
    _In_ size_t        OutputBufferLength,
    _In_ size_t        InputBufferLength,
    _In_ ULONG         IoControlCode
)
{
    WDFDEVICE               hDevice;
    PDEVICE_EXTENSION       devExt;
    NTSTATUS                status = STATUS_SUCCESS;
    WDF_REQUEST_SEND_OPTIONS options;
    BOOLEAN                 ret;

    UNREFERENCED_PARAMETER(OutputBufferLength);
    UNREFERENCED_PARAMETER(InputBufferLength);

    hDevice = WdfIoQueueGetDevice(Queue);
    devExt = FilterGetData(hDevice);

    switch (IoControlCode) {

    case IOCTL_INTERNAL_KEYBOARD_CONNECT:
        {
            PCONNECT_DATA connectData;
            size_t length;

            // kbfiltr guard: only allow one connection per device
            if (devExt->UpperConnectData.ClassService != NULL) {
                AnyKeyDebugPrint("CONNECT DeviceId=%lu: already connected\n",
                                 devExt->DeviceId);
                status = STATUS_SHARING_VIOLATION;
                break;
            }

            status = WdfRequestRetrieveInputBuffer(
                Request, sizeof(CONNECT_DATA), &connectData, &length);
            if (!NT_SUCCESS(status)) {
                AnyKeyDebugPrint("CONNECT: RetrieveInputBuffer failed 0x%x\n", status);
                break;
            }

            // Save original + hook our callback (same as kbfiltr)
            devExt->UpperConnectData = *connectData;
            connectData->ClassDeviceObject = WdfDeviceWdmGetDeviceObject(hDevice);
#pragma warning(disable:4152)
            connectData->ClassService = KbFilter_ServiceCallback;
#pragma warning(default:4152)

            AnyKeyDebugPrint("IOCTL_INTERNAL_KEYBOARD_CONNECT hooked DeviceId=%lu\n",
                             devExt->DeviceId);
        }
        break;

    case IOCTL_INTERNAL_KEYBOARD_DISCONNECT:
        devExt->UpperConnectData.ClassService = NULL;
        AnyKeyDebugPrint("IOCTL_INTERNAL_KEYBOARD_DISCONNECT DeviceId=%lu\n",
                         devExt->DeviceId);
        break;

    default:
        // All other internal IOCTLs: pass through to port driver
        break;
    }

    //
    // If the switch set an error, complete the request here.
    //
    if (!NT_SUCCESS(status)) {
        WdfRequestComplete(Request, status);
        return;
    }

    //
    // Forward the (possibly modified) request down to the port driver.
    // This is the critical step — the port driver must receive CONNECT
    // with our hooked ClassService, otherwise ServiceCallback never fires.
    //
    WDF_REQUEST_SEND_OPTIONS_INIT(&options,
                                  WDF_REQUEST_SEND_OPTION_SEND_AND_FORGET);

    ret = WdfRequestSend(Request, WdfDeviceGetIoTarget(hDevice), &options);
    if (ret == FALSE) {
        status = WdfRequestGetStatus(Request);
        AnyKeyDebugPrint("WdfRequestSend(internal IOCTL) failed: 0x%x\n", status);
        WdfRequestComplete(Request, status);
    }
}

// ===============================================================
// MouFilter_EvtIoInternalDeviceControl (v0.2)
// Same pattern as KbFilter but handles MOUSE_CONNECT/DISCONNECT.
// Hooks mouclass callback, forwards everything to the port driver.
// ===============================================================

static VOID
MouFilter_EvtIoInternalDeviceControl(
    _In_ WDFQUEUE      Queue,
    _In_ WDFREQUEST    Request,
    _In_ size_t        OutputBufferLength,
    _In_ size_t        InputBufferLength,
    _In_ ULONG         IoControlCode
)
{
    WDFDEVICE               hDevice;
    PDEVICE_EXTENSION       devExt;
    NTSTATUS                status = STATUS_SUCCESS;
    WDF_REQUEST_SEND_OPTIONS options;
    BOOLEAN                 ret;

    UNREFERENCED_PARAMETER(OutputBufferLength);
    UNREFERENCED_PARAMETER(InputBufferLength);

    hDevice = WdfIoQueueGetDevice(Queue);
    devExt = FilterGetData(hDevice);

    switch (IoControlCode) {

    case IOCTL_INTERNAL_MOUSE_CONNECT:
        {
            PCONNECT_DATA connectData;
            size_t length;

            if (devExt->MouseConnectData.ClassService != NULL) {
                AnyKeyDebugPrint("MOUSE_CONNECT DeviceId=%lu: already connected\n",
                                 devExt->DeviceId);
                status = STATUS_SHARING_VIOLATION;
                break;
            }

            status = WdfRequestRetrieveInputBuffer(
                Request, sizeof(CONNECT_DATA), &connectData, &length);
            if (!NT_SUCCESS(status)) {
                AnyKeyDebugPrint("MOUSE_CONNECT: RetrieveInputBuffer failed 0x%x\n", status);
                break;
            }

            // Save original + hook our callback
            devExt->MouseConnectData = *connectData;
            connectData->ClassDeviceObject = WdfDeviceWdmGetDeviceObject(hDevice);
#pragma warning(disable:4152)
            connectData->ClassService = MouFilter_ServiceCallback;
#pragma warning(default:4152)

            AnyKeyDebugPrint("IOCTL_INTERNAL_MOUSE_CONNECT hooked DeviceId=%lu\n",
                             devExt->DeviceId);
        }
        break;

    case IOCTL_INTERNAL_MOUSE_DISCONNECT:
        devExt->MouseConnectData.ClassService = NULL;
        AnyKeyDebugPrint("IOCTL_INTERNAL_MOUSE_DISCONNECT DeviceId=%lu\n",
                         devExt->DeviceId);
        break;

    default:
        // Pass through to port driver
        break;
    }

    if (!NT_SUCCESS(status)) {
        WdfRequestComplete(Request, status);
        return;
    }

    WDF_REQUEST_SEND_OPTIONS_INIT(&options,
                                  WDF_REQUEST_SEND_OPTION_SEND_AND_FORGET);

    ret = WdfRequestSend(Request, WdfDeviceGetIoTarget(hDevice), &options);
    if (ret == FALSE) {
        status = WdfRequestGetStatus(Request);
        AnyKeyDebugPrint("WdfRequestSend(mouse internal IOCTL) failed: 0x%x\n", status);
        WdfRequestComplete(Request, status);
    }
}

// ===============================================================
// KbFilter_ServiceCallback — hooked into kbdclass callback chain
// Called at DISPATCH_LEVEL for each keyboard input packet.
// ===============================================================

VOID
KbFilter_ServiceCallback(
    _In_     PVOID    NormalContext,
    _In_     PVOID    SystemArgument1,
    _In_     PVOID    SystemArgument2,
    _Inout_  PVOID    SystemArgument3
)
{
    // WDK 10.0.28000+ uses opaque PVOID — cast to concrete types
    PDEVICE_OBJECT       DeviceObject     = (PDEVICE_OBJECT)NormalContext;
    PKEYBOARD_INPUT_DATA InputDataStart   = (PKEYBOARD_INPUT_DATA)SystemArgument1;
    PKEYBOARD_INPUT_DATA InputDataEnd     = (PKEYBOARD_INPUT_DATA)SystemArgument2;
    PULONG               InputDataConsumed = (PULONG)SystemArgument3;

    // Diagnostic: print every N invocations to confirm callback is alive
    static LONG cbCnt = 0;
    LONG n = InterlockedIncrement(&cbCnt);
    if ((n & 0x7F) == 1) {  // every 128th call
        DbgPrint("[AnyKeyFlt] ServiceCallback #%d devObj=%p\n", n, DeviceObject);
    }

    WDFDEVICE           hDevice;
    PDEVICE_EXTENSION   devExt;
    ULONG               i = 0;
    ULONG               count;
    KIRQL               oldIrql = { 0 };

    hDevice = WdfWdmDeviceGetWdfDeviceHandle(DeviceObject);
    devExt = FilterGetData(hDevice);
    count = (ULONG)(InputDataEnd - InputDataStart);

    // ===========================================================
    // EMERGENCY COMBO CHECK — highest priority, before heartbeat.
    // Detect LCtrl(0x1D)+Space(0x39)+Esc(0x01) all held down.
    // Runs at DISPATCH_LEVEL; single BOOLEAN reads/writes are atomic.
    // Excludes E0-prefixed variants (RCtrl also=0x1D but with E0).
    // ===========================================================
    {
        ULONG ei;
        for (ei = 0; ei < count; ei++) {
            USHORT mk  = InputDataStart[ei].MakeCode;
            USHORT flg = InputDataStart[ei].Flags;
            BOOLEAN is_make = ((flg & KEY_BREAK) == 0);
            BOOLEAN is_e0   = ((flg & KEY_E0)  != 0);

            // Track only the non-E0 variants (L* keys, not R* keys)
            if (mk == 0x1D && !is_e0) {
                g_AnyKey.Emergency_Lctrl = is_make;
            } else if (mk == 0x39 && !is_e0) {
                g_AnyKey.Emergency_Space = is_make;
            } else if (mk == 0x01 && !is_e0) {
                g_AnyKey.Emergency_Esc   = is_make;
            }

            // Emergency: all three held simultaneously
            if (g_AnyKey.Emergency_Lctrl &&
                g_AnyKey.Emergency_Space &&
                g_AnyKey.Emergency_Esc) {

                DbgPrint("[AnyKeyFlt] EMERGENCY COMBO: LCtrl+Space+Esc -> shutdown\n");

                // Reset emergency state BEFORE shutdown (avoids re-entry)
                g_AnyKey.Emergency_Lctrl = FALSE;
                g_AnyKey.Emergency_Space = FALSE;
                g_AnyKey.Emergency_Esc   = FALSE;

                // Release all held modifiers + mouse buttons, flush queues,
                // disable interception on ALL devices.
                AnyKey_EmergencyShutdown();

                // Swallow this packet — the three emergency keys are consumed.
                *InputDataConsumed = count;
                return;
            }
        }
    }

    //
    // Session+Heartbeat watchdog: if this device is being intercepted
    // and engine has a session but hasn't sent a heartbeat → emergency shutdown.
    // (Check any device's InterceptEnabled — if any is ON, engine should be alive.)
    //
    if (devExt->InterceptEnabled && g_AnyKey.SessionActive &&
        g_AnyKey.LastHeartbeat.QuadPart != 0) {
        LARGE_INTEGER now;
        now.QuadPart = KeQueryInterruptTime();
        LONGLONG elapsed = (now.QuadPart - g_AnyKey.LastHeartbeat.QuadPart);
        // Convert 100ns units to seconds (1 second = 10,000,000 * 100ns)
        elapsed /= 10000000LL;
        if (elapsed > ANYKEY_HEARTBEAT_TIMEOUT_SECONDS) {
            AnyKeyDebugPrint("HEARTBEAT: timeout (%lld sec) -> emergency shutdown\n", elapsed);
            AnyKey_EmergencyShutdown();
        }
    }

    //
    // per-device pass-through: forward to original callback, do NOT enqueue
    //
    if (!devExt->InterceptEnabled) {
        if (devExt->UpperConnectData.ClassService != NULL) {
            PSERVICE_CALLBACK_ROUTINE callback =
                (PSERVICE_CALLBACK_ROUTINE)devExt->UpperConnectData.ClassService;
            callback(
                devExt->UpperConnectData.ClassDeviceObject,
                (PVOID)InputDataStart,
                (PVOID)InputDataEnd,
                (PVOID)InputDataConsumed
            );
        }
        // Passthrough capture: mirror into queue without consuming
        if (g_AnyKey.CaptureEnabled) {
            KeAcquireSpinLock(&devExt->InputQueueLock, &oldIrql);
            for (i = 0; i < count; i++) {
                if (devExt->InputQueueCount >= ANYKEY_MAX_INPUT_QUEUE) {
                    devExt->InputQueueHead = (devExt->InputQueueHead + 1) % ANYKEY_MAX_INPUT_QUEUE;
                    devExt->InputQueueCount--;
                }
                PANYKEY_INPUT_EVENT evt = &devExt->InputQueue[devExt->InputQueueTail];
                evt->MakeCode  = InputDataStart[i].MakeCode;
                evt->Flags     = InputDataStart[i].Flags;
                evt->DeviceId  = devExt->DeviceId;
                evt->ExtraInfo = InputDataStart[i].ExtraInformation;
                devExt->InputQueueTail = (devExt->InputQueueTail + 1) % ANYKEY_MAX_INPUT_QUEUE;
                devExt->InputQueueCount++;
            }
            KeReleaseSpinLock(&devExt->InputQueueLock, oldIrql);
        }
        return;
    }

    //
    // Intercept mode: copy each KEYBOARD_INPUT_DATA into the queue.
    //
    KeAcquireSpinLock(&devExt->InputQueueLock, &oldIrql);

    for (i = 0; i < count; i++) {
        if (devExt->InputQueueCount >= ANYKEY_MAX_INPUT_QUEUE) {
            // Queue full — drop oldest entry
            devExt->InputQueueHead = (devExt->InputQueueHead + 1) % ANYKEY_MAX_INPUT_QUEUE;
            devExt->InputQueueCount--;
        }

        PANYKEY_INPUT_EVENT evt = &devExt->InputQueue[devExt->InputQueueTail];
        evt->MakeCode  = InputDataStart[i].MakeCode;
        evt->Flags     = InputDataStart[i].Flags;
        evt->DeviceId  = devExt->DeviceId;
        evt->ExtraInfo = InputDataStart[i].ExtraInformation;

        devExt->InputQueueTail = (devExt->InputQueueTail + 1) % ANYKEY_MAX_INPUT_QUEUE;
        devExt->InputQueueCount++;
    }

    KeReleaseSpinLock(&devExt->InputQueueLock, oldIrql);

    // Signal the user-mode event (if registered) that input is available.
    // KeSetEvent with IO_NO_INCREMENT is safe at DISPATCH_LEVEL.
    if (g_AnyKey.hInputEvent != NULL) {
        KeSetEvent(g_AnyKey.hInputEvent, IO_NO_INCREMENT, FALSE);
    }

    // Consume all input — block from reaching system
    *InputDataConsumed = count;
}

// ===============================================================
// MouFilter_ServiceCallback (v0.2) — hooked into mouclass callback chain
// Called at DISPATCH_LEVEL for each MOUSE_INPUT_DATA packet.
// Same pattern as KbFilter_ServiceCallback but processes mouse data.
// ===============================================================

VOID
MouFilter_ServiceCallback(
    _In_     PVOID    NormalContext,
    _In_     PVOID    SystemArgument1,
    _In_     PVOID    SystemArgument2,
    _Inout_  PVOID    SystemArgument3
)
{
    PDEVICE_OBJECT       DeviceObject     = (PDEVICE_OBJECT)NormalContext;
    PMOUSE_INPUT_DATA    InputDataStart   = (PMOUSE_INPUT_DATA)SystemArgument1;
    PMOUSE_INPUT_DATA    InputDataEnd     = (PMOUSE_INPUT_DATA)SystemArgument2;
    PULONG               InputDataConsumed = (PULONG)SystemArgument3;

    static LONG mouCbCnt = 0;
    LONG n = InterlockedIncrement(&mouCbCnt);
    if ((n & 0xFF) == 1) {  // every 256th call
        DbgPrint("[AnyKeyFlt] MouServiceCallback #%d devObj=%p\n", n, DeviceObject);
    }

    WDFDEVICE           hDevice;
    PDEVICE_EXTENSION   devExt;
    ULONG               i;
    ULONG               count;
    KIRQL               oldIrql;

    hDevice = WdfWdmDeviceGetWdfDeviceHandle(DeviceObject);
    devExt = FilterGetData(hDevice);
    count = (ULONG)(InputDataEnd - InputDataStart);

    //
    // Session+Heartbeat watchdog (shared with keyboard -- same global state).
    //
    if (devExt->InterceptEnabled && g_AnyKey.SessionActive &&
        g_AnyKey.LastHeartbeat.QuadPart != 0) {
        LARGE_INTEGER now;
        now.QuadPart = KeQueryInterruptTime();
        LONGLONG elapsed = (now.QuadPart - g_AnyKey.LastHeartbeat.QuadPart);
        elapsed /= 10000000LL;
        if (elapsed > ANYKEY_HEARTBEAT_TIMEOUT_SECONDS) {
            AnyKeyDebugPrint("MOUSE HEARTBEAT: timeout (%lld sec) -> emergency shutdown\n", elapsed);
            AnyKey_EmergencyShutdown();
        }
    }

    //
    // per-device pass-through: forward to original mouclass callback
    //
    if (!devExt->InterceptEnabled) {
        if (devExt->MouseConnectData.ClassService != NULL) {
            PSERVICE_CALLBACK_ROUTINE callback =
                (PSERVICE_CALLBACK_ROUTINE)devExt->MouseConnectData.ClassService;
            callback(
                devExt->MouseConnectData.ClassDeviceObject,
                (PVOID)InputDataStart,
                (PVOID)InputDataEnd,
                (PVOID)InputDataConsumed
            );
        }
        // Passthrough capture: mirror mouse input into the MouseQueue for
        // passive identification WITHOUT consuming it (see keyboard path).
        if (g_AnyKey.CaptureEnabled) {
            KeAcquireSpinLock(&devExt->MouseQueueLock, &oldIrql);
            for (i = 0; i < count; i++) {
                PMOUSE_INPUT_DATA mid = &InputDataStart[i];
                if (devExt->MouseQueueCount >= ANYKEY_MAX_INPUT_QUEUE) {
                    devExt->MouseQueueHead = (devExt->MouseQueueHead + 1) % ANYKEY_MAX_INPUT_QUEUE;
                    devExt->MouseQueueCount--;
                }
                PANYKEY_MOUSE_EVENT evt = &devExt->MouseQueue[devExt->MouseQueueTail];
                evt->DeviceId    = devExt->DeviceId;
                evt->Flags       = mid->Flags;
                evt->ButtonFlags = mid->ButtonFlags;
                evt->ButtonData  = (SHORT)mid->ButtonData;
                evt->LastX       = mid->LastX;
                evt->LastY       = mid->LastY;
                evt->ExtraInfo   = mid->ExtraInformation;
                devExt->MouseQueueTail = (devExt->MouseQueueTail + 1) % ANYKEY_MAX_INPUT_QUEUE;
                devExt->MouseQueueCount++;
            }
            KeReleaseSpinLock(&devExt->MouseQueueLock, oldIrql);
        }
        return;
    }

    //
    // Intercept mode: split by packet type.
    // Pure movement (ButtonFlags==0): forward to system immediately, no queue.
    // Button/wheel events: queue in MouseQueue for engine processing.
    //
    {
        BOOLEAN queuedAnything = FALSE;
        KeAcquireSpinLock(&devExt->MouseQueueLock, &oldIrql);

        for (i = 0; i < count; i++) {
            PMOUSE_INPUT_DATA mid = &InputDataStart[i];

            // Pure movement (no button, no wheel)
            if (mid->ButtonFlags == 0) {
                // Always forward to system — cursor must move regardless
                if (devExt->MouseConnectData.ClassService != NULL) {
                    PSERVICE_CALLBACK_ROUTINE callback =
                        (PSERVICE_CALLBACK_ROUTINE)devExt->MouseConnectData.ClassService;
                    ULONG one = 0;
                    callback(
                        devExt->MouseConnectData.ClassDeviceObject,
                        (PVOID)mid,
                        (PVOID)(mid + 1),
                        (PVOID)&one
                    );
                }

                // If gesture mode active: queue movement for engine too
                if (g_AnyKey.MouseGesturing) {
                    if (devExt->MouseQueueCount >= ANYKEY_MAX_INPUT_QUEUE) {
                        devExt->MouseQueueHead = (devExt->MouseQueueHead + 1) % ANYKEY_MAX_INPUT_QUEUE;
                        devExt->MouseQueueCount--;
                    }
                    PANYKEY_MOUSE_EVENT evt = &devExt->MouseQueue[devExt->MouseQueueTail];
                    evt->DeviceId    = devExt->DeviceId;
                    evt->Flags       = mid->Flags;
                    evt->ButtonFlags = mid->ButtonFlags;
                    evt->ButtonData  = (SHORT)mid->ButtonData;
                    evt->LastX       = mid->LastX;
                    evt->LastY       = mid->LastY;
                    evt->ExtraInfo   = mid->ExtraInformation;
                    devExt->MouseQueueTail = (devExt->MouseQueueTail + 1) % ANYKEY_MAX_INPUT_QUEUE;
                    devExt->MouseQueueCount++;
                    queuedAnything = TRUE;
                }
                continue;
            }

            // Button or wheel: queue for engine
            if (devExt->MouseQueueCount >= ANYKEY_MAX_INPUT_QUEUE) {
                // Queue full — drop oldest entry
                devExt->MouseQueueHead = (devExt->MouseQueueHead + 1) % ANYKEY_MAX_INPUT_QUEUE;
                devExt->MouseQueueCount--;
            }

            PANYKEY_MOUSE_EVENT evt = &devExt->MouseQueue[devExt->MouseQueueTail];

            evt->DeviceId    = devExt->DeviceId;
            evt->Flags       = mid->Flags;
            evt->ButtonFlags = mid->ButtonFlags;
            evt->ButtonData  = (SHORT)mid->ButtonData;
            evt->LastX       = mid->LastX;
            evt->LastY       = mid->LastY;
            evt->ExtraInfo   = mid->ExtraInformation;

            devExt->MouseQueueTail = (devExt->MouseQueueTail + 1) % ANYKEY_MAX_INPUT_QUEUE;
            devExt->MouseQueueCount++;
            queuedAnything = TRUE;
        }

        KeReleaseSpinLock(&devExt->MouseQueueLock, oldIrql);

        // Signal event only if we actually queued button/wheel events.
        // Pure movement packets are forwarded directly, no engine wakeup needed.
        if (queuedAnything && g_AnyKey.hInputEvent != NULL) {
            KeSetEvent(g_AnyKey.hInputEvent, IO_NO_INCREMENT, FALSE);
        }
    }

    // Consume all mouse input — block from reaching system
    *InputDataConsumed = count;
}

// ===============================================================
// Helper: drain input queues from all devices into user buffer
// ===============================================================

static NTSTATUS
AnyKey_DrainInputQueues(
    _Out_writes_bytes_to_(OutputBufferLength, *BytesReturned)
         PVOID        OutputBuffer,
    _In_  size_t      OutputBufferLength,
    _Out_ size_t*     BytesReturned
)
{
    KIRQL       oldIrql;
    PLIST_ENTRY entry;
    size_t      maxEvents = OutputBufferLength / sizeof(ANYKEY_INPUT_EVENT);
    size_t      written = 0;
    PANYKEY_INPUT_EVENT outBuf = (PANYKEY_INPUT_EVENT)OutputBuffer;

    if (OutputBuffer == NULL || OutputBufferLength < sizeof(ANYKEY_INPUT_EVENT)) {
        *BytesReturned = 0;
        return STATUS_BUFFER_TOO_SMALL;
    }

    // Acquire global list lock (raises IRQL to DISPATCH_LEVEL)
    KeAcquireSpinLock(&g_AnyKey.ListLock, &oldIrql);

    for (entry = g_AnyKey.DeviceListHead.Flink;
         entry != &g_AnyKey.DeviceListHead && written < maxEvents;
         entry = entry->Flink)
    {
        PDEVICE_EXTENSION devExt = CONTAINING_RECORD(entry, DEVICE_EXTENSION, ListEntry);

        // Inner spinlock: we're already at DISPATCH_LEVEL from the outer lock
        KeAcquireSpinLockAtDpcLevel(&devExt->InputQueueLock);

        while (devExt->InputQueueCount > 0 && written < maxEvents) {
            outBuf[written] = devExt->InputQueue[devExt->InputQueueHead];
            devExt->InputQueueHead = (devExt->InputQueueHead + 1) % ANYKEY_MAX_INPUT_QUEUE;
            devExt->InputQueueCount--;
            written++;
        }

        // Release inner lock without lowering IRQL (outer lock still held)
        KeReleaseSpinLockFromDpcLevel(&devExt->InputQueueLock);
    }

    // Release global lock (lowers IRQL back to PASSIVE_LEVEL)
    KeReleaseSpinLock(&g_AnyKey.ListLock, oldIrql);

    *BytesReturned = written * sizeof(ANYKEY_INPUT_EVENT);
    // Always return STATUS_SUCCESS — 0 bytes means "no data available".
    // This is cleaner than STATUS_NO_MORE_ENTRIES (avoids Win32 error mapping).
    return STATUS_SUCCESS;
}

// ===============================================================
// Helper: inject output by calling original kbdclass callback
// ===============================================================

static NTSTATUS
AnyKey_InjectOutput(
    _In_ PANYKEY_OUTPUT_EVENT OutputEvent
)
{
    KIRQL           oldIrql;
    PLIST_ENTRY     entry;
    CONNECT_DATA    savedConnectData;
    RtlZeroMemory(&savedConnectData, sizeof(savedConnectData));
    BOOLEAN         found = FALSE;

    KeAcquireSpinLock(&g_AnyKey.ListLock, &oldIrql);

    for (entry = g_AnyKey.DeviceListHead.Flink;
         entry != &g_AnyKey.DeviceListHead;
         entry = entry->Flink)
    {
        PDEVICE_EXTENSION devExt = CONTAINING_RECORD(entry, DEVICE_EXTENSION, ListEntry);

        // Skip mouse devices — keyboard output injection only
        if (devExt->DeviceType == ANYKEY_DEV_MOUSE) continue;

        if (OutputEvent->DeviceId == 0 || devExt->DeviceId == OutputEvent->DeviceId) {
            if (devExt->UpperConnectData.ClassService != NULL) {
                savedConnectData = devExt->UpperConnectData;
                found = TRUE;
            }
            break;
        }
    }

    KeReleaseSpinLock(&g_AnyKey.ListLock, oldIrql);

    if (!found) {
        AnyKeyDebugPrint("AnyKey_InjectOutput: no target device (devId=%lu)\n",
                         OutputEvent->DeviceId);
        return STATUS_NOT_FOUND;
    }

    KEYBOARD_INPUT_DATA kibData;
    RtlZeroMemory(&kibData, sizeof(kibData));
    kibData.MakeCode = OutputEvent->MakeCode;
    kibData.Flags    = OutputEvent->Flags;

    ULONG consumed = 0;

    PSERVICE_CALLBACK_ROUTINE callback =
        (PSERVICE_CALLBACK_ROUTINE)savedConnectData.ClassService;
    callback(
        savedConnectData.ClassDeviceObject,
        (PVOID)&kibData,
        (PVOID)(&kibData + 1),
        (PVOID)&consumed
    );

    AnyKeyDebugPrint("InjectOutput: makecode=0x%02x flags=0x%02x\n",
                     OutputEvent->MakeCode, OutputEvent->Flags);

    return STATUS_SUCCESS;
}

// ===============================================================
// Helper: drain mouse input queues into user buffer (v0.2)
// Mirrors AnyKey_DrainInputQueues but for MouseQueue/ANYKEY_MOUSE_EVENT.
// ===============================================================

static NTSTATUS
AnyKey_DrainMouseQueues(
    _Out_writes_bytes_to_(OutputBufferLength, *BytesReturned)
         PVOID        OutputBuffer,
    _In_  size_t      OutputBufferLength,
    _Out_ size_t*     BytesReturned
)
{
    KIRQL       oldIrql;
    PLIST_ENTRY entry;
    size_t      maxEvents = OutputBufferLength / sizeof(ANYKEY_MOUSE_EVENT);
    size_t      written = 0;
    PANYKEY_MOUSE_EVENT outBuf = (PANYKEY_MOUSE_EVENT)OutputBuffer;

    if (OutputBuffer == NULL || OutputBufferLength < sizeof(ANYKEY_MOUSE_EVENT)) {
        *BytesReturned = 0;
        return STATUS_BUFFER_TOO_SMALL;
    }

    KeAcquireSpinLock(&g_AnyKey.ListLock, &oldIrql);

    for (entry = g_AnyKey.DeviceListHead.Flink;
         entry != &g_AnyKey.DeviceListHead && written < maxEvents;
         entry = entry->Flink)
    {
        PDEVICE_EXTENSION devExt = CONTAINING_RECORD(entry, DEVICE_EXTENSION, ListEntry);

        // Only drain mouse devices
        if (devExt->DeviceType != ANYKEY_DEV_MOUSE) continue;

        KeAcquireSpinLockAtDpcLevel(&devExt->MouseQueueLock);

        while (devExt->MouseQueueCount > 0 && written < maxEvents) {
            outBuf[written] = devExt->MouseQueue[devExt->MouseQueueHead];
            devExt->MouseQueueHead = (devExt->MouseQueueHead + 1) % ANYKEY_MAX_INPUT_QUEUE;
            devExt->MouseQueueCount--;
            written++;
        }

        KeReleaseSpinLockFromDpcLevel(&devExt->MouseQueueLock);
    }

    KeReleaseSpinLock(&g_AnyKey.ListLock, oldIrql);

    *BytesReturned = written * sizeof(ANYKEY_MOUSE_EVENT);
    return STATUS_SUCCESS;
}

// ===============================================================
// Helper: inject mouse output by calling original mouclass callback (v0.2)
// ===============================================================

static NTSTATUS
AnyKey_InjectMouseOutput(
    _In_ PANYKEY_MOUSE_OUTPUT_EVENT OutputEvent
)
{
    KIRQL           oldIrql;
    PLIST_ENTRY     entry;
    CONNECT_DATA    savedConnectData;
    RtlZeroMemory(&savedConnectData, sizeof(savedConnectData));
    BOOLEAN         found = FALSE;

    KeAcquireSpinLock(&g_AnyKey.ListLock, &oldIrql);

    for (entry = g_AnyKey.DeviceListHead.Flink;
         entry != &g_AnyKey.DeviceListHead;
         entry = entry->Flink)
    {
        PDEVICE_EXTENSION devExt = CONTAINING_RECORD(entry, DEVICE_EXTENSION, ListEntry);

        if (devExt->DeviceType != ANYKEY_DEV_MOUSE) continue;

        if (OutputEvent->DeviceId == 0 || devExt->DeviceId == OutputEvent->DeviceId) {
            if (devExt->MouseConnectData.ClassService != NULL) {
                savedConnectData = devExt->MouseConnectData;
                found = TRUE;
            }
            break;
        }
    }

    KeReleaseSpinLock(&g_AnyKey.ListLock, oldIrql);

    if (!found) {
        AnyKeyDebugPrint("InjectMouseOutput: no target mouse (devId=%lu)\n",
                         OutputEvent->DeviceId);
        return STATUS_NOT_FOUND;
    }

    MOUSE_INPUT_DATA mid;
    RtlZeroMemory(&mid, sizeof(mid));
    mid.Flags       = OutputEvent->Flags;
    mid.ButtonFlags = OutputEvent->ButtonFlags;
    mid.ButtonData  = (USHORT)OutputEvent->ButtonData;
    mid.LastX       = OutputEvent->LastX;
    mid.LastY       = OutputEvent->LastY;

    ULONG consumed = 0;

    PSERVICE_CALLBACK_ROUTINE callback =
        (PSERVICE_CALLBACK_ROUTINE)savedConnectData.ClassService;
    callback(
        savedConnectData.ClassDeviceObject,
        (PVOID)&mid,
        (PVOID)(&mid + 1),
        (PVOID)&consumed
    );

    AnyKeyDebugPrint("InjectMouseOutput: btnFlags=0x%04x data=%d x=%ld y=%ld\n",
                     OutputEvent->ButtonFlags, OutputEvent->ButtonData,
                     OutputEvent->LastX, OutputEvent->LastY);

    return STATUS_SUCCESS;
}

// ===============================================================
// AnyKey_EmergencyShutdown — heartbeat timeout recovery.
// Disables interception, flushes all device queues, and
// injects key-up events for common modifiers (Ctrl/Shift/Alt/Win)
// so users aren't stuck with held keys after recovery.
// ===============================================================

static VOID
AnyKey_EmergencyShutdown(VOID)
{
    KIRQL oldIrql;
    PLIST_ENTRY entry;

    // 1. Disable interception on ALL devices (v0.4: per-device)
    KeAcquireSpinLock(&g_AnyKey.ListLock, &oldIrql);
    for (entry = g_AnyKey.DeviceListHead.Flink;
         entry != &g_AnyKey.DeviceListHead;
         entry = entry->Flink) {
        PDEVICE_EXTENSION devExt = CONTAINING_RECORD(entry, DEVICE_EXTENSION, ListEntry);
        KIRQL qlIrql;

        // Disable interception for this device
        devExt->InterceptEnabled = FALSE;

        // Flush keyboard queue
        KeAcquireSpinLock(&devExt->InputQueueLock, &qlIrql);
        devExt->InputQueueHead = 0;
        devExt->InputQueueTail = 0;
        devExt->InputQueueCount = 0;
        KeReleaseSpinLock(&devExt->InputQueueLock, qlIrql);

        // Flush mouse queue
        KeAcquireSpinLock(&devExt->MouseQueueLock, &qlIrql);
        devExt->MouseQueueHead = 0;
        devExt->MouseQueueTail = 0;
        devExt->MouseQueueCount = 0;
        KeReleaseSpinLock(&devExt->MouseQueueLock, qlIrql);
    }
    KeReleaseSpinLock(&g_AnyKey.ListLock, oldIrql);

    // 3. Inject KEY_BREAK for all modifiers through kbdclass callback
    //    to release any keys held when engine died.
    //    Uses (MakeCode, Flags) pairs — critical for E0-prefixed keys:
    //    RCtrl=0x1D+E0, RAlt=0x38+E0, Win keys=5B/5C+E0.
    static const struct { USHORT mk; USHORT flg; } BREAK_ENTRIES[] = {
        { 0x1D, KEY_BREAK },               // LCtrl
        { 0x1D, (USHORT)(KEY_BREAK | KEY_E0) }, // RCtrl (E0 prefix)
        { 0x2A, KEY_BREAK },               // LShift
        { 0x36, KEY_BREAK },               // RShift
        { 0x38, KEY_BREAK },               // LAlt
        { 0x38, (USHORT)(KEY_BREAK | KEY_E0) }, // RAlt (E0 prefix)
        { 0x5B, (USHORT)(KEY_BREAK | KEY_E0) }, // LWin (E0 prefix)
        { 0x5C, (USHORT)(KEY_BREAK | KEY_E0) }, // RWin (E0 prefix)
    };

    for (entry = g_AnyKey.DeviceListHead.Flink;
         entry != &g_AnyKey.DeviceListHead;
         entry = entry->Flink) {
        PDEVICE_EXTENSION devExt = CONTAINING_RECORD(entry, DEVICE_EXTENSION, ListEntry);
        if (devExt->UpperConnectData.ClassService == NULL) continue;

        CONNECT_DATA savedData = devExt->UpperConnectData;
        PSERVICE_CALLBACK_ROUTINE callback =
            (PSERVICE_CALLBACK_ROUTINE)savedData.ClassService;

        for (SIZE_T i = 0; i < sizeof(BREAK_ENTRIES)/sizeof(BREAK_ENTRIES[0]); i++) {
            KEYBOARD_INPUT_DATA kib;
            RtlZeroMemory(&kib, sizeof(kib));
            kib.MakeCode = BREAK_ENTRIES[i].mk;
            kib.Flags    = BREAK_ENTRIES[i].flg;
            ULONG consumed = 0;
            callback(
                savedData.ClassDeviceObject,
                (PVOID)&kib,
                (PVOID)(&kib + 1),
                (PVOID)&consumed
            );
        }
    }

    // 4. Inject MOUSE_BUTTON_UP for common mouse buttons (v0.2)
    //    to release any buttons held when engine died.
    {
        static const USHORT MOUSE_RELEASE_FLAGS[] = {
            MOUSE_LEFT_BUTTON_UP,
            MOUSE_RIGHT_BUTTON_UP,
            MOUSE_MIDDLE_BUTTON_UP,
            MOUSE_BUTTON_4_UP,   // XButton1
            MOUSE_BUTTON_5_UP,   // XButton2
        };

        KeAcquireSpinLock(&g_AnyKey.ListLock, &oldIrql);
        for (entry = g_AnyKey.DeviceListHead.Flink;
             entry != &g_AnyKey.DeviceListHead;
             entry = entry->Flink) {
            PDEVICE_EXTENSION devExt = CONTAINING_RECORD(entry, DEVICE_EXTENSION, ListEntry);
            if (devExt->DeviceType != ANYKEY_DEV_MOUSE) continue;
            if (devExt->MouseConnectData.ClassService == NULL) continue;

            CONNECT_DATA savedData = devExt->MouseConnectData;
            PSERVICE_CALLBACK_ROUTINE callback =
                (PSERVICE_CALLBACK_ROUTINE)savedData.ClassService;

            for (SIZE_T i = 0; i < sizeof(MOUSE_RELEASE_FLAGS)/sizeof(MOUSE_RELEASE_FLAGS[0]); i++) {
                MOUSE_INPUT_DATA mid;
                RtlZeroMemory(&mid, sizeof(mid));
                mid.Flags       = MOUSE_MOVE_RELATIVE;
                mid.ButtonFlags = MOUSE_RELEASE_FLAGS[i];
                ULONG consumed = 0;
                callback(
                    savedData.ClassDeviceObject,
                    (PVOID)&mid,
                    (PVOID)(&mid + 1),
                    (PVOID)&consumed
                );
            }
        }
        KeReleaseSpinLock(&g_AnyKey.ListLock, oldIrql);
    }

    // 5. Reset session state
    g_AnyKey.SessionActive = FALSE;
    g_AnyKey.LastHeartbeat.QuadPart = 0;

    AnyKeyDebugPrint("EMERGENCY: intercept disabled, queues flushed, modifiers released\n");
}

// ===============================================================
// EvtIoDeviceControlFromRawPdo — IOCTL handler for user-mode
// ===============================================================

VOID
KbFilter_EvtIoDeviceControlFromRawPdo(
    _In_ WDFQUEUE      Queue,
    _In_ WDFREQUEST    Request,
    _In_ size_t        OutputBufferLength,
    _In_ size_t        InputBufferLength,
    _In_ ULONG         IoControlCode
)
{
    NTSTATUS            status = STATUS_SUCCESS;
    WDFDEVICE           hDevice;
    PDEVICE_EXTENSION   devExt;
    size_t              bytesReturned = 0;
    WDFMEMORY           outMem;
    PVOID               outBuf, inBuf;

    UNREFERENCED_PARAMETER(InputBufferLength);

    hDevice = WdfIoQueueGetDevice(Queue);
    devExt = FilterGetData(hDevice);

    switch (IoControlCode) {

    // ── IOCTL_ANYKEY_WAIT_INPUT (non-blocking poll) ──
    case IOCTL_ANYKEY_WAIT_INPUT:
        {
            status = WdfRequestRetrieveOutputMemory(Request, &outMem);
            if (!NT_SUCCESS(status)) {
                AnyKeyDebugPrint("WAIT_INPUT: RetrieveOutputMemory failed 0x%x\n", status);
                break;
            }

            outBuf = WdfMemoryGetBuffer(outMem, NULL);

            status = AnyKey_DrainInputQueues(outBuf, OutputBufferLength, &bytesReturned);

            if (NT_SUCCESS(status)) {
                AnyKeyDebugPrint("WAIT_INPUT: returned %Iu events\n",
                                 bytesReturned / sizeof(ANYKEY_INPUT_EVENT));
            }
        }
        break;

    // ── IOCTL_ANYKEY_WAIT_MOUSE_INPUT (v0.2 — non-blocking poll for mouse) ──
    case IOCTL_ANYKEY_WAIT_MOUSE_INPUT:
        {
            status = WdfRequestRetrieveOutputMemory(Request, &outMem);
            if (!NT_SUCCESS(status)) {
                AnyKeyDebugPrint("WAIT_MOUSE_INPUT: RetrieveOutputMemory failed 0x%x\n", status);
                break;
            }

            outBuf = WdfMemoryGetBuffer(outMem, NULL);

            status = AnyKey_DrainMouseQueues(outBuf, OutputBufferLength, &bytesReturned);

            if (NT_SUCCESS(status)) {
                AnyKeyDebugPrint("WAIT_MOUSE_INPUT: returned %Iu events\n",
                                 bytesReturned / sizeof(ANYKEY_MOUSE_EVENT));
            }
        }
        break;

    // ── IOCTL_ANYKEY_SEND_OUTPUT ──
    case IOCTL_ANYKEY_SEND_OUTPUT:
        {
            if (InputBufferLength < sizeof(ANYKEY_OUTPUT_EVENT)) {
                status = STATUS_BUFFER_TOO_SMALL;
                break;
            }

            status = WdfRequestRetrieveInputBuffer(Request, sizeof(ANYKEY_OUTPUT_EVENT),
                                                    &inBuf, NULL);
            if (!NT_SUCCESS(status)) {
                AnyKeyDebugPrint("SEND_OUTPUT: RetrieveInputBuffer failed 0x%x\n", status);
                break;
            }

            status = AnyKey_InjectOutput((PANYKEY_OUTPUT_EVENT)inBuf);
            bytesReturned = 0;
        }
        break;

    // ── IOCTL_ANYKEY_SEND_MOUSE_OUTPUT (v0.2) ──
    case IOCTL_ANYKEY_SEND_MOUSE_OUTPUT:
        {
            if (InputBufferLength < sizeof(ANYKEY_MOUSE_OUTPUT_EVENT)) {
                status = STATUS_BUFFER_TOO_SMALL;
                break;
            }

            status = WdfRequestRetrieveInputBuffer(Request, sizeof(ANYKEY_MOUSE_OUTPUT_EVENT),
                                                    &inBuf, NULL);
            if (!NT_SUCCESS(status)) {
                AnyKeyDebugPrint("SEND_MOUSE_OUTPUT: RetrieveInputBuffer failed 0x%x\n", status);
                break;
            }

            status = AnyKey_InjectMouseOutput((PANYKEY_MOUSE_OUTPUT_EVENT)inBuf);
            bytesReturned = 0;
        }
        break;

    // ── IOCTL_ANYKEY_GET_DEVICE_COUNT ──
    case IOCTL_ANYKEY_GET_DEVICE_COUNT:
        {
            if (OutputBufferLength < sizeof(ULONG)) {
                status = STATUS_BUFFER_TOO_SMALL;
                break;
            }

            KIRQL oldIrql;
            ULONG deviceCount = 0;
            PLIST_ENTRY entry;

            KeAcquireSpinLock(&g_AnyKey.ListLock, &oldIrql);
            for (entry = g_AnyKey.DeviceListHead.Flink;
                 entry != &g_AnyKey.DeviceListHead;
                 entry = entry->Flink)
            {
                deviceCount++;
            }
            KeReleaseSpinLock(&g_AnyKey.ListLock, oldIrql);

            status = WdfRequestRetrieveOutputMemory(Request, &outMem);
            if (NT_SUCCESS(status)) {
                WdfMemoryCopyFromBuffer(outMem, 0, &deviceCount, sizeof(ULONG));
                bytesReturned = sizeof(ULONG);
            }
        }
        break;

    // ── IOCTL_ANYKEY_GET_DEVICE_INFO ──
    case IOCTL_ANYKEY_GET_DEVICE_INFO:
        {
            if (InputBufferLength < sizeof(ULONG) ||
                OutputBufferLength < sizeof(ANYKEY_DEVICE_INFO)) {
                status = STATUS_BUFFER_TOO_SMALL;
                break;
            }

            status = WdfRequestRetrieveInputBuffer(Request, sizeof(ULONG),
                                                    &inBuf, NULL);
            if (!NT_SUCCESS(status)) break;

            ULONG targetId = *(PULONG)inBuf;

            KIRQL oldIrql;
            PLIST_ENTRY entry;
            BOOLEAN foundDev = FALSE;
            ANYKEY_DEVICE_INFO devInfo;
            RtlZeroMemory(&devInfo, sizeof(devInfo));

            KeAcquireSpinLock(&g_AnyKey.ListLock, &oldIrql);
            for (entry = g_AnyKey.DeviceListHead.Flink;
                 entry != &g_AnyKey.DeviceListHead;
                 entry = entry->Flink)
            {
                PDEVICE_EXTENSION listDevExt =
                    CONTAINING_RECORD(entry, DEVICE_EXTENSION, ListEntry);
                if (listDevExt->DeviceId == targetId) {
                    devInfo.DeviceId = listDevExt->DeviceId;
                    devInfo.IsKeyboard = (listDevExt->DeviceType == ANYKEY_DEV_KEYBOARD);
                    devInfo.IsMouse    = (listDevExt->DeviceType == ANYKEY_DEV_MOUSE);
                    RtlStringCbCopyW(devInfo.HardwareId, sizeof(devInfo.HardwareId),
                                     listDevExt->HardwareId);
                    RtlStringCbCopyW(devInfo.ContainerId, sizeof(devInfo.ContainerId),
                                     listDevExt->ContainerId);
                    RtlStringCbCopyW(devInfo.FriendlyName, sizeof(devInfo.FriendlyName),
                                     listDevExt->FriendlyName);
                    devInfo.VendorId  = listDevExt->VendorId;
                    devInfo.ProductId = listDevExt->ProductId;
                    devInfo.HasSerialNumber = listDevExt->HasSerialNumber;
                    devInfo.Flags     = listDevExt->DeviceFlags;
                    foundDev = TRUE;
                    break;
                }
            }
            KeReleaseSpinLock(&g_AnyKey.ListLock, oldIrql);

            if (!foundDev) {
                status = STATUS_NOT_FOUND;
                break;
            }

            status = WdfRequestRetrieveOutputMemory(Request, &outMem);
            if (NT_SUCCESS(status)) {
                WdfMemoryCopyFromBuffer(outMem, 0, &devInfo, sizeof(devInfo));
                bytesReturned = sizeof(devInfo);
            }
        }
        break;

    // ── IOCTL_ANYKEY_SET_INTERCEPT (v0.4: per-device) ──
    case IOCTL_ANYKEY_SET_INTERCEPT:
        {
            if (InputBufferLength < sizeof(ANYKEY_INTERCEPT_REQUEST)) {
                status = STATUS_BUFFER_TOO_SMALL;
                break;
            }

            ANYKEY_INTERCEPT_REQUEST req;
            status = WdfRequestRetrieveInputBuffer(Request, sizeof(req),
                                                    &inBuf, NULL);
            if (!NT_SUCCESS(status)) break;
            RtlCopyMemory(&req, inBuf, sizeof(req));

            KIRQL oldIrql;
            PLIST_ENTRY entry;
            ULONG changed = 0;

            KeAcquireSpinLock(&g_AnyKey.ListLock, &oldIrql);
            for (entry = g_AnyKey.DeviceListHead.Flink;
                 entry != &g_AnyKey.DeviceListHead;
                 entry = entry->Flink) {
                PDEVICE_EXTENSION iterExt =
                    CONTAINING_RECORD(entry, DEVICE_EXTENSION, ListEntry);
                if (req.DeviceId == 0 || iterExt->DeviceId == req.DeviceId) {
                    iterExt->InterceptEnabled = req.Enable;
                    changed++;
                    if (req.DeviceId != 0) break; // specific device found
                }
            }
            KeReleaseSpinLock(&g_AnyKey.ListLock, oldIrql);

            if (req.Enable) {
                // Session starts: reset heartbeat, mark session active
                g_AnyKey.SessionActive = TRUE;
                g_AnyKey.LastHeartbeat.QuadPart = KeQueryInterruptTime();
            }

            AnyKeyDebugPrint("SET_INTERCEPT: dev=%lu %s (%lu devices)\n",
                             req.DeviceId,
                             req.Enable ? "INTERCEPT" : "PASSTHROUGH",
                             changed);
            bytesReturned = 0;
        }
        break;

    // ── IOCTL_ANYKEY_SET_MOUSE_MOVE (v0.2 — gesture tracking toggle) ──
    case IOCTL_ANYKEY_SET_MOUSE_MOVE:
        {
            if (InputBufferLength < sizeof(BOOLEAN)) {
                status = STATUS_BUFFER_TOO_SMALL;
                break;
            }

            status = WdfRequestRetrieveInputBuffer(Request, sizeof(BOOLEAN),
                                                    &inBuf, NULL);
            if (!NT_SUCCESS(status)) break;

            BOOLEAN enable = *(PBOOLEAN)inBuf;
            g_AnyKey.MouseGesturing = enable;

            AnyKeyDebugPrint("SET_MOUSE_MOVE: %s\n", enable ? "GESTURE ON" : "GESTURE OFF");
            bytesReturned = 0;
        }
        break;

    // ── IOCTL_ANYKEY_SET_CAPTURE ──
    case IOCTL_ANYKEY_SET_CAPTURE:
        {
            if (InputBufferLength < sizeof(BOOLEAN)) {
                status = STATUS_BUFFER_TOO_SMALL;
                break;
            }

            status = WdfRequestRetrieveInputBuffer(Request, sizeof(BOOLEAN),
                                                    &inBuf, NULL);
            if (!NT_SUCCESS(status)) break;

            BOOLEAN cap = *(PBOOLEAN)inBuf;
            g_AnyKey.CaptureEnabled = cap;

            AnyKeyDebugPrint("SET_CAPTURE: %s\n", cap ? "ENABLED" : "DISABLED");
            bytesReturned = 0;
        }
        break;

    // ── IOCTL_ANYKEY_SET_EVENT ──
    case IOCTL_ANYKEY_SET_EVENT:
        {
            if (InputBufferLength < sizeof(HANDLE)) {
                status = STATUS_BUFFER_TOO_SMALL;
                break;
            }

            status = WdfRequestRetrieveInputBuffer(Request, sizeof(HANDLE),
                                                    &inBuf, NULL);
            if (!NT_SUCCESS(status)) break;

            HANDLE hUserEvent = *(PHANDLE)inBuf;

            // Release previous event if any
            if (g_AnyKey.hInputEventObject != NULL) {
                ObDereferenceObject(g_AnyKey.hInputEventObject);
                g_AnyKey.hInputEventObject = NULL;
                g_AnyKey.hInputEvent = NULL;
            }

            if (hUserEvent != NULL) {
                status = ObReferenceObjectByHandle(
                    hUserEvent,
                    EVENT_MODIFY_STATE,
                    *ExEventObjectType,
                    KernelMode,
                    &g_AnyKey.hInputEventObject,
                    NULL);

                if (NT_SUCCESS(status)) {
                    g_AnyKey.hInputEvent = (PKEVENT)g_AnyKey.hInputEventObject;
                    AnyKeyDebugPrint("SET_EVENT: registered user-mode event\n");
                } else {
                    AnyKeyDebugPrint("SET_EVENT: ObReferenceObjectByHandle failed 0x%x\n", status);
                    g_AnyKey.hInputEventObject = NULL;
                }
            } else {
                // NULL handle = unregister
                AnyKeyDebugPrint("SET_EVENT: unregistered\n");
            }

            bytesReturned = 0;
        }
        break;

    // ── IOCTL_ANYKEY_HEARTBEAT ──
    case IOCTL_ANYKEY_HEARTBEAT:
        {
            g_AnyKey.SessionActive = TRUE;
            g_AnyKey.LastHeartbeat.QuadPart = KeQueryInterruptTime();

            // Optionally return driver status on heartbeat
            PANYKEY_HEARTBEAT_RESPONSE resp;
            size_t respSize = sizeof(ANYKEY_HEARTBEAT_RESPONSE);
            if (OutputBufferLength >= respSize) {
                status = WdfRequestRetrieveOutputBuffer(Request, respSize,
                                                        &outBuf, NULL);
                if (NT_SUCCESS(status)) {
                    resp = (PANYKEY_HEARTBEAT_RESPONSE)outBuf;
                    RtlZeroMemory(resp, respSize);
                    resp->DriverVersion = ANYKEY_DRIVER_VERSION;
                    resp->Timestamp     = g_AnyKey.LastHeartbeat;

                    // Count queued events across all devices
                    ULONG totalQueued = 0;
                    ULONG deviceCount = 0;
                    KIRQL qlIrql;
                    KeAcquireSpinLock(&g_AnyKey.ListLock, &qlIrql);
                    PLIST_ENTRY e;
                    for (e = g_AnyKey.DeviceListHead.Flink;
                         e != &g_AnyKey.DeviceListHead;
                         e = e->Flink) {
                        PDEVICE_EXTENSION dExt = CONTAINING_RECORD(e, DEVICE_EXTENSION, ListEntry);
                        totalQueued += dExt->InputQueueCount;
                        totalQueued += dExt->MouseQueueCount;
                        deviceCount++;
                    }
                    KeReleaseSpinLock(&g_AnyKey.ListLock, qlIrql);

                    resp->QueueDepth  = totalQueued;
                    resp->DeviceCount = deviceCount;
                    // v0.4: per-device — state_flags based on is any device intercepting
                    {
                        BOOLEAN anyIntercepting = FALSE;
                        for (e = g_AnyKey.DeviceListHead.Flink;
                             e != &g_AnyKey.DeviceListHead;
                             e = e->Flink) {
                            PDEVICE_EXTENSION dExt =
                                CONTAINING_RECORD(e, DEVICE_EXTENSION, ListEntry);
                            if (dExt->InterceptEnabled) {
                                anyIntercepting = TRUE;
                                break;
                            }
                        }
                        resp->StateFlags = anyIntercepting
                                         ? ANYKEY_STATE_INTERCEPTING
                                         : ANYKEY_STATE_HEALTHY;
                    }
                    bytesReturned = respSize;
                }
            } else {
                bytesReturned = 0;
            }
            status = STATUS_SUCCESS;
        }
        break;

    // ── IOCTL_ANYKEY_ENUM_DEVICES ──
    case IOCTL_ANYKEY_ENUM_DEVICES:
        {
            ANYKEY_ENUM_DEVICES_REQUEST req;
            status = WdfRequestRetrieveInputBuffer(Request, sizeof(req), &inBuf, NULL);
            if (!NT_SUCCESS(status)) break;
            RtlCopyMemory(&req, inBuf, sizeof(req));

            size_t maxOut = OutputBufferLength / sizeof(ANYKEY_DEVICE_INFO);
            if (maxOut > req.MaxCount) maxOut = req.MaxCount;
            if (maxOut == 0) { bytesReturned = 0; status = STATUS_SUCCESS; break; }

            status = WdfRequestRetrieveOutputBuffer(Request,
                maxOut * sizeof(ANYKEY_DEVICE_INFO), &outBuf, NULL);
            if (!NT_SUCCESS(status)) break;

            PANYKEY_DEVICE_INFO out = (PANYKEY_DEVICE_INFO)outBuf;
            ULONG written = 0;
            ULONG skip = req.Index;

            KIRQL oldIrql;
            PLIST_ENTRY entry;
            KeAcquireSpinLock(&g_AnyKey.ListLock, &oldIrql);
            for (entry = g_AnyKey.DeviceListHead.Flink;
                 entry != &g_AnyKey.DeviceListHead && written < maxOut;
                 entry = entry->Flink)
            {
                if (skip > 0) { skip--; continue; }
                PDEVICE_EXTENSION listDevExt = CONTAINING_RECORD(entry, DEVICE_EXTENSION, ListEntry);
                RtlZeroMemory(&out[written], sizeof(ANYKEY_DEVICE_INFO));
                out[written].DeviceId   = listDevExt->DeviceId;
                out[written].IsKeyboard = (listDevExt->DeviceType == ANYKEY_DEV_KEYBOARD);
                out[written].IsMouse    = (listDevExt->DeviceType == ANYKEY_DEV_MOUSE);
                RtlStringCbCopyW(out[written].HardwareId, sizeof(out[written].HardwareId), listDevExt->HardwareId);
                RtlStringCbCopyW(out[written].ContainerId, sizeof(out[written].ContainerId), listDevExt->ContainerId);
                RtlStringCbCopyW(out[written].FriendlyName, sizeof(out[written].FriendlyName), listDevExt->FriendlyName);
                out[written].VendorId      = listDevExt->VendorId;
                out[written].ProductId     = listDevExt->ProductId;
                out[written].HasSerialNumber = listDevExt->HasSerialNumber;
                out[written].Flags          = listDevExt->DeviceFlags;
                written++;
            }
            KeReleaseSpinLock(&g_AnyKey.ListLock, oldIrql);
            bytesReturned = written * sizeof(ANYKEY_DEVICE_INFO);
            status = STATUS_SUCCESS;
        }
        break;

    // ── IOCTL_ANYKEY_GET_STATUS (v0.4: per-device counters) ──
    case IOCTL_ANYKEY_GET_STATUS:
        {
            if (OutputBufferLength < sizeof(ANYKEY_DRIVER_STATUS)) {
                status = STATUS_BUFFER_TOO_SMALL;
                break;
            }
            status = WdfRequestRetrieveOutputBuffer(Request,
                sizeof(ANYKEY_DRIVER_STATUS), &outBuf, NULL);
            if (!NT_SUCCESS(status)) break;

            PANYKEY_DRIVER_STATUS st = (PANYKEY_DRIVER_STATUS)outBuf;
            RtlZeroMemory(st, sizeof(ANYKEY_DRIVER_STATUS));
            {
                KIRQL oldIrql;
                KeAcquireSpinLock(&g_AnyKey.ListLock, &oldIrql);
                ULONG cnt = 0, icnt = 0;
                PLIST_ENTRY e;
                for (e = g_AnyKey.DeviceListHead.Flink;
                     e != &g_AnyKey.DeviceListHead; e = e->Flink) {
                    PDEVICE_EXTENSION dExt =
                        CONTAINING_RECORD(e, DEVICE_EXTENSION, ListEntry);
                    cnt++;
                    if (dExt->InterceptEnabled) icnt++;
                }
                st->DeviceCount = cnt;
                st->InterceptingCount = icnt;
                KeReleaseSpinLock(&g_AnyKey.ListLock, oldIrql);
            }
            st->Flags = 0;
            if (g_AnyKey.DeviceListChanged) {
                st->Flags |= ANYKEY_FLAG_DEVICE_CHANGED;
                g_AnyKey.DeviceListChanged = FALSE;
            }
            bytesReturned = sizeof(ANYKEY_DRIVER_STATUS);
            status = STATUS_SUCCESS;
        }
        break;

    default:
        status = STATUS_NOT_IMPLEMENTED;
        break;
    }

    WdfRequestCompleteWithInformation(Request, status, bytesReturned);
}
