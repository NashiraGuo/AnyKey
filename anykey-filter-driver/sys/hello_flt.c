/*++
  AnyKey Hello Filter — minimal filter, zero logic.
  Only hooks callback and forwards everything.
  Uses WDM IRP preprocess for CONNECT (avoids default queue conflict
  in class filters).
--*/

#include <ntddk.h>
#include <wdf.h>
#include <kbdmou.h>

typedef struct _DEVICE_EXTENSION {
    CONNECT_DATA UpperConnectData;
} DEVICE_EXTENSION, *PDEVICE_EXTENSION;

WDF_DECLARE_CONTEXT_TYPE_WITH_NAME(DEVICE_EXTENSION, FilterGetData)

#define DebugPrint(x, ...) DbgPrintEx(DPFLTR_IHVDRIVER_ID, DPFLTR_ERROR_LEVEL, \
    "[AnyKeyHello] " x, __VA_ARGS__)

// ── Pass-through service callback ──
VOID Hello_ServiceCallback(
    PVOID NormalContext, PVOID SystemArgument1,
    PVOID SystemArgument2, PVOID SystemArgument3)
{
    PDEVICE_OBJECT DeviceObject = (PDEVICE_OBJECT)NormalContext;
    WDFDEVICE hDevice = WdfWdmDeviceGetWdfDeviceHandle(DeviceObject);
    PDEVICE_EXTENSION devExt = FilterGetData(hDevice);

    if (devExt->UpperConnectData.ClassService != NULL) {
        PSERVICE_CALLBACK_ROUTINE cb =
            (PSERVICE_CALLBACK_ROUTINE)devExt->UpperConnectData.ClassService;
        cb(NormalContext, SystemArgument1, SystemArgument2, SystemArgument3);
    }
}

// ── WDM IRP preprocess for CONNECT/DISCONNECT ──
static NTSTATUS Hello_PreprocessInternalIoctl(
    WDFDEVICE Device, PIRP Irp)
{
    PIO_STACK_LOCATION irpSp = IoGetCurrentIrpStackLocation(Irp);
    ULONG ioControlCode = irpSp->Parameters.DeviceIoControl.IoControlCode;
    PDEVICE_EXTENSION devExt = FilterGetData(Device);

    if (ioControlCode != IOCTL_INTERNAL_KEYBOARD_CONNECT &&
        ioControlCode != IOCTL_INTERNAL_KEYBOARD_DISCONNECT) {
        return WdfDeviceWdmDispatchPreprocessedIrp(Device, Irp);
    }

    if (ioControlCode == IOCTL_INTERNAL_KEYBOARD_CONNECT) {
        PCONNECT_DATA connectData =
            (PCONNECT_DATA)irpSp->Parameters.DeviceIoControl.Type3InputBuffer;

        if (connectData != NULL) {
            devExt->UpperConnectData = *connectData;
            connectData->ClassDeviceObject = WdfDeviceWdmGetDeviceObject(Device);
            connectData->ClassService = (PVOID)Hello_ServiceCallback;
            DebugPrint("CONNECT hooked\n");
        }
        // CONNECT handled — complete it, don't forward
        Irp->IoStatus.Status = STATUS_SUCCESS;
        Irp->IoStatus.Information = 0;
        IoCompleteRequest(Irp, IO_NO_INCREMENT);
        return STATUS_NOT_SUPPORTED; // Tell WDF we handled it
    } else {
        devExt->UpperConnectData.ClassService = NULL;
        Irp->IoStatus.Status = STATUS_SUCCESS;
        Irp->IoStatus.Information = 0;
        IoCompleteRequest(Irp, IO_NO_INCREMENT);
        return STATUS_NOT_SUPPORTED;
    }
}

// ── EvtDeviceAdd ──
NTSTATUS Hello_EvtDeviceAdd(WDFDRIVER Driver, PWDFDEVICE_INIT DeviceInit)
{
    UNREFERENCED_PARAMETER(Driver);
    WdfFdoInitSetFilter(DeviceInit);

    // Register preprocess BEFORE WdfDeviceCreate
    WdfDeviceInitAssignWdmIrpPreprocessCallback(
        DeviceInit, Hello_PreprocessInternalIoctl,
        IRP_MJ_INTERNAL_DEVICE_CONTROL, NULL, 0);

    DebugPrint("EvtDeviceAdd enter\n");

    WDF_OBJECT_ATTRIBUTES attr;
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&attr, DEVICE_EXTENSION);

    WDFDEVICE hDevice;
    NTSTATUS status = WdfDeviceCreate(&DeviceInit, &attr, &hDevice);
    if (!NT_SUCCESS(status)) {
        DebugPrint("WdfDeviceCreate failed 0x%x\n", status);
        return status;
    }

    PDEVICE_EXTENSION devExt = FilterGetData(hDevice);
    RtlZeroMemory(devExt, sizeof(DEVICE_EXTENSION));

    DebugPrint("EvtDeviceAdd success\n");
    return STATUS_SUCCESS;
}

// ── DriverEntry ──
NTSTATUS DriverEntry(PDRIVER_OBJECT DriverObject, PUNICODE_STRING RegistryPath)
{
    DebugPrint("Hello Filter Driver v0.1.0\n");
    WDF_DRIVER_CONFIG cfg;
    WDF_DRIVER_CONFIG_INIT(&cfg, Hello_EvtDeviceAdd);
    return WdfDriverCreate(DriverObject, RegistryPath,
                           WDF_NO_OBJECT_ATTRIBUTES, &cfg, WDF_NO_HANDLE);
}
