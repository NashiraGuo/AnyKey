/*++
  AnyKey Filter Driver -- rawpdo.c
  Creates a RawPDO per filter device instance for sideband user-mode
  communication. The keyboard device is exclusive/secure, so we create
  a separate device node that applications can open and send custom IOCTLs to.

  Based on Microsoft kbfiltr sample rawpdo.c.
--*/

#include "anykey_flt.h"

//
// Device ID for the RawPDO
//
#define ANYKEY_RAWPDO_DEVICE_ID L"AnyKey_RawPDO\0"

//
// SDDL: SYSTEM and Administrators only -- keyboard is a secure device
//
static CONST UNICODE_STRING g_SDDL =
    RTL_CONSTANT_STRING(L"D:P(A;;GA;;;SY)(A;;GA;;;BA)");

//
// Forward declarations
//
EVT_WDF_IO_QUEUE_IO_DEVICE_CONTROL
    KbFilter_EvtIoDeviceControlForRawPdo;

#define MAX_ID_LEN 128

//
// EvtIoDeviceControlForRawPdo -- just forwards to parent's queue
//
VOID
KbFilter_EvtIoDeviceControlForRawPdo(
    _In_ WDFQUEUE      Queue,
    _In_ WDFREQUEST    Request,
    _In_ size_t        OutputBufferLength,
    _In_ size_t        InputBufferLength,
    _In_ ULONG         IoControlCode
)
{
    WDFDEVICE                   parent = WdfIoQueueGetDevice(Queue);
    PRPDO_DEVICE_DATA           pdoData;
    WDF_REQUEST_FORWARD_OPTIONS forwardOptions;
    NTSTATUS                    status = STATUS_SUCCESS;

    UNREFERENCED_PARAMETER(OutputBufferLength);
    UNREFERENCED_PARAMETER(InputBufferLength);

    pdoData = PdoGetData(parent);

    switch (IoControlCode) {

    case IOCTL_ANYKEY_WAIT_INPUT:
    case IOCTL_ANYKEY_SEND_OUTPUT:
    case IOCTL_ANYKEY_GET_DEVICE_COUNT:
    case IOCTL_ANYKEY_GET_DEVICE_INFO:
    case IOCTL_ANYKEY_SET_INTERCEPT:
        //
        // Forward to the parent filter device's RawPDO queue
        //
        WDF_REQUEST_FORWARD_OPTIONS_INIT(&forwardOptions);
        status = WdfRequestForwardToParentDeviceIoQueue(
            Request, pdoData->ParentQueue, &forwardOptions);
        if (!NT_SUCCESS(status)) {
            WdfRequestComplete(Request, status);
        }
        break;

    default:
        WdfRequestComplete(Request, STATUS_NOT_IMPLEMENTED);
        break;
    }
}

//
// KbFiltr_CreateRawPdo -- create a raw PDO child device for sideband
// communication with user-mode applications.
//
NTSTATUS
KbFiltr_CreateRawPdo(
    _In_ WDFDEVICE   Device,
    _In_ ULONG       InstanceNo
)
{
    NTSTATUS                    status;
    PWDFDEVICE_INIT             pDeviceInit = NULL;
    PRPDO_DEVICE_DATA           pdoData = NULL;
    WDFDEVICE                   hChild = NULL;
    WDF_OBJECT_ATTRIBUTES       pdoAttributes;
    WDF_DEVICE_PNP_CAPABILITIES pnpCaps;
    WDF_IO_QUEUE_CONFIG         ioQueueConfig;
    WDFQUEUE                    queue;
    WDF_DEVICE_STATE            deviceState;
    PDEVICE_EXTENSION           devExt;
    DECLARE_CONST_UNICODE_STRING(deviceId, ANYKEY_RAWPDO_DEVICE_ID);
    DECLARE_CONST_UNICODE_STRING(deviceLocation, L"AnyKey Filter\0");
    DECLARE_UNICODE_STRING_SIZE(buffer, MAX_ID_LEN);

    AnyKeyDebugPrint("KbFiltr_CreateRawPdo: instance=%lu\n", InstanceNo);

    pDeviceInit = WdfPdoInitAllocate(Device);
    if (pDeviceInit == NULL) {
        status = STATUS_INSUFFICIENT_RESOURCES;
        goto Cleanup;
    }

    // WdfPdoInitAssignRawDevice(pDeviceInit, &GUID_DEVCLASS_KEYBOARD) removed.
    // Assigning GUID_DEVCLASS_KEYBOARD to a raw PDO child of an UpperFilter
    // causes START_DEVICE to fail with STATUS_INVALID_DEVICE_STATE (0xC0000184)
    // because the parent is not a full keyboard bus. The child does not need
    // a class GUID -- it is discovered solely via GUID_DEVINTERFACE_ANYKEY_FLT.

    //
    // Restrict access to SYSTEM and Administrators
    //
    status = WdfDeviceInitAssignSDDLString(pDeviceInit, &g_SDDL);
    if (!NT_SUCCESS(status)) goto Cleanup;

    //
    // Assign device ID
    //
    status = WdfPdoInitAssignDeviceID(pDeviceInit, &deviceId);
    if (!NT_SUCCESS(status)) goto Cleanup;

    //
    // Provide instance ID to avoid bugcheck on multiple instances
    //
    status = RtlUnicodeStringPrintf(&buffer, L"%02lu", InstanceNo);
    if (!NT_SUCCESS(status)) goto Cleanup;

    status = WdfPdoInitAssignInstanceID(pDeviceInit, &buffer);
    if (!NT_SUCCESS(status)) goto Cleanup;

    //
    // Device description (shown in Device Manager if not hidden)
    //
    status = RtlUnicodeStringPrintf(&buffer, L"AnyKey_Filter_%02lu", InstanceNo);
    if (!NT_SUCCESS(status)) goto Cleanup;

    status = WdfPdoInitAddDeviceText(pDeviceInit, &buffer, &deviceLocation, 0x409);
    if (!NT_SUCCESS(status)) goto Cleanup;

    WdfPdoInitSetDefaultLocale(pDeviceInit, 0x409);

    //
    // Allow forwarding requests to parent
    //
    WdfPdoInitAllowForwardingRequestToParent(pDeviceInit);

    //
    // Create the child device
    //
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&pdoAttributes, RPDO_DEVICE_DATA);

    status = WdfDeviceCreate(&pDeviceInit, &pdoAttributes, &hChild);
    if (!NT_SUCCESS(status)) goto Cleanup;

    pdoData = PdoGetData(hChild);
    pdoData->InstanceNo = InstanceNo;

    devExt = FilterGetData(Device);
    pdoData->ParentQueue = devExt->RawPdoQueue;

    //
    // Default queue for the RawPDO (forwards to parent)
    //
    WDF_IO_QUEUE_CONFIG_INIT_DEFAULT_QUEUE(&ioQueueConfig, WdfIoQueueDispatchSequential);
    ioQueueConfig.EvtIoDeviceControl = KbFilter_EvtIoDeviceControlForRawPdo;

    status = WdfIoQueueCreate(hChild, &ioQueueConfig, WDF_NO_OBJECT_ATTRIBUTES, &queue);
    if (!NT_SUCCESS(status)) {
        AnyKeyDebugPrint("WdfIoQueueCreate(rawPdo default) failed: 0x%x\n", status);
        goto Cleanup;
    }

    //
    // PnP capabilities: removable, hidden from UI
    //
    WDF_DEVICE_PNP_CAPABILITIES_INIT(&pnpCaps);
    pnpCaps.Removable         = WdfTrue;
    pnpCaps.SurpriseRemovalOK = WdfTrue;
    pnpCaps.NoDisplayInUI     = WdfTrue;
    pnpCaps.Address           = InstanceNo;
    pnpCaps.UINumber          = InstanceNo;
    WdfDeviceSetPnpCapabilities(hChild, &pnpCaps);

    //
    // Hide from Device Manager
    //
    WDF_DEVICE_STATE_INIT(&deviceState);
    deviceState.DontDisplayInUI = WdfTrue;
    WdfDeviceSetDeviceState(hChild, &deviceState);

    //
    // Register device interface so user mode can discover and open it
    //
    status = WdfDeviceCreateDeviceInterface(hChild, &GUID_DEVINTERFACE_ANYKEY_FLT, NULL);
    if (!NT_SUCCESS(status)) {
        AnyKeyDebugPrint("WdfDeviceCreateDeviceInterface failed: 0x%x\n", status);
        goto Cleanup;
    }

    //
    // Add as static child of the filter device
    //
    status = WdfFdoAddStaticChild(Device, hChild);
    if (!NT_SUCCESS(status)) goto Cleanup;

    AnyKeyDebugPrint("KbFiltr_CreateRawPdo: success, instance=%lu\n", InstanceNo);
    return STATUS_SUCCESS;

Cleanup:
    AnyKeyDebugPrint("KbFiltr_CreateRawPdo failed: 0x%x\n", status);

    if (pDeviceInit != NULL) {
        WdfDeviceInitFree(pDeviceInit);
    }
    if (hChild) {
        WdfObjectDelete(hChild);
    }
    return status;
}
