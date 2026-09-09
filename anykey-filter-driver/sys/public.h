/*++
  AnyKey Filter Driver -- public.h
  IOCTL definitions shared between driver and user-mode engine.
  Based on Microsoft kbfiltr sample architecture.
--*/

#ifndef _ANKEY_FLT_PUBLIC_H
#define _ANKEY_FLT_PUBLIC_H

// -- Device interface GUID for user-mode discovery --
// {A1B2C3D4-E5F6-7890-ABCD-EF1234567890}
DEFINE_GUID(GUID_DEVINTERFACE_ANYKEY_FLT,
    0xa1b2c3d4, 0xe5f6, 0x7890, 0xab, 0xcd,
    0xef, 0x12, 0x34, 0x56, 0x78, 0x90);

// -- Device ID --
#define ANYKEY_FLT_DEVICE_ID L"AnyKey_Filter\0"

// -- IOCTL codes --
// FILE_DEVICE_KEYBOARD = 0x0000000b
#define ANYKEY_IOCTL_INDEX             0x900

// IOCTL_ANYKEY_WAIT_INPUT -- Non-blocking poll for input events.
// Output: ANYKEY_INPUT_EVENT[] (0 or more events per call)
// Returns STATUS_SUCCESS with bytesReturned=0 if no data available.
#define IOCTL_ANYKEY_WAIT_INPUT        CTL_CODE(FILE_DEVICE_KEYBOARD,  \
                                               ANYKEY_IOCTL_INDEX,     \
                                               METHOD_BUFFERED,         \
                                               FILE_READ_DATA | FILE_WRITE_DATA)

// IOCTL_ANYKEY_SEND_OUTPUT -- Inject keyboard/mouse output.
// Input: ANYKEY_OUTPUT_EVENT
#define IOCTL_ANYKEY_SEND_OUTPUT       CTL_CODE(FILE_DEVICE_KEYBOARD,  \
                                               ANYKEY_IOCTL_INDEX + 1, \
                                               METHOD_BUFFERED,         \
                                               FILE_READ_DATA | FILE_WRITE_DATA)

// IOCTL_ANYKEY_GET_DEVICE_COUNT -- Get number of filtered devices.
// Output: ULONG
#define IOCTL_ANYKEY_GET_DEVICE_COUNT  CTL_CODE(FILE_DEVICE_KEYBOARD,  \
                                               ANYKEY_IOCTL_INDEX + 2, \
                                               METHOD_BUFFERED,         \
                                               FILE_READ_DATA)

// IOCTL_ANYKEY_GET_DEVICE_INFO -- Get hardware ID for a device by index.
// Input: ULONG (device index)
// Output: ANYKEY_DEVICE_INFO
#define IOCTL_ANYKEY_GET_DEVICE_INFO   CTL_CODE(FILE_DEVICE_KEYBOARD,  \
                                               ANYKEY_IOCTL_INDEX + 3, \
                                               METHOD_BUFFERED,         \
                                               FILE_READ_DATA | FILE_WRITE_DATA)

// IOCTL_ANYKEY_SET_INTERCEPT -- Enable/disable per-device input interception.
// Input: ANYKEY_INTERCEPT_REQUEST
//   DeviceId=0: apply to all devices; DeviceId=N: apply to device N only.
//   Enable=TRUE: intercept (queue + consume); Enable=FALSE: pass-through.
#define IOCTL_ANYKEY_SET_INTERCEPT     CTL_CODE(FILE_DEVICE_KEYBOARD,  \
                                               ANYKEY_IOCTL_INDEX + 4, \
                                               METHOD_BUFFERED,         \
                                               FILE_WRITE_DATA)

// IOCTL_ANYKEY_SET_EVENT -- Register a user-mode event handle for input notification.
// Input: HANDLE (user-mode event object from CreateEventW).
#define IOCTL_ANYKEY_SET_EVENT         CTL_CODE(FILE_DEVICE_KEYBOARD,  \
                                               ANYKEY_IOCTL_INDEX + 5, \
                                               METHOD_BUFFERED,         \
                                               FILE_WRITE_DATA)

// IOCTL_ANYKEY_HEARTBEAT -- Health check from engine watchdog thread.
#define IOCTL_ANYKEY_HEARTBEAT        CTL_CODE(FILE_DEVICE_KEYBOARD,  \
                                               ANYKEY_IOCTL_INDEX + 6, \
                                               METHOD_BUFFERED,         \
                                               FILE_READ_DATA)

// IOCTL_ANYKEY_ENUM_DEVICES -- Enumerate all filter devices.
// Input: ANYKEY_ENUM_DEVICES_REQUEST
// Output: ANYKEY_DEVICE_INFO[] (up to MaxCount, starting from Index)
#define IOCTL_ANYKEY_ENUM_DEVICES     CTL_CODE(FILE_DEVICE_KEYBOARD,  \
                                               ANYKEY_IOCTL_INDEX + 7, \
                                               METHOD_BUFFERED,         \
                                               FILE_READ_DATA | FILE_WRITE_DATA)

// IOCTL_ANYKEY_GET_STATUS -- Query driver global status.
// Output: ANYKEY_DRIVER_STATUS
// Auto-clears volatile flags (DEVICE_CHANGED) after read.
#define IOCTL_ANYKEY_GET_STATUS       CTL_CODE(FILE_DEVICE_KEYBOARD,  \
                                               ANYKEY_IOCTL_INDEX + 8, \
                                               METHOD_BUFFERED,         \
                                               FILE_READ_DATA)

// -- Mouse IOCTLs (v0.2) --

// IOCTL_ANYKEY_WAIT_MOUSE_INPUT -- Non-blocking poll for mouse input events.
// Output: ANYKEY_MOUSE_EVENT[] (0 or more events per call)
// Returns STATUS_SUCCESS with bytesReturned=0 if no data available.
#define IOCTL_ANYKEY_WAIT_MOUSE_INPUT  CTL_CODE(FILE_DEVICE_KEYBOARD,  \
                                               ANYKEY_IOCTL_INDEX + 9,  \
                                               METHOD_BUFFERED,         \
                                               FILE_READ_DATA | FILE_WRITE_DATA)

// IOCTL_ANYKEY_SEND_MOUSE_OUTPUT -- Inject mouse output.
// Input: ANYKEY_MOUSE_OUTPUT_EVENT
#define IOCTL_ANYKEY_SEND_MOUSE_OUTPUT CTL_CODE(FILE_DEVICE_KEYBOARD,  \
                                               ANYKEY_IOCTL_INDEX + 10, \
                                               METHOD_BUFFERED,         \
                                               FILE_READ_DATA | FILE_WRITE_DATA)

// IOCTL_ANYKEY_SET_MOUSE_MOVE -- Enable/disable mouse movement queuing (v0.2).
// Input: BOOLEAN (TRUE = queue+signal movement events for gesture tracking,
//                  FALSE = forward-only, movement events not queued)
// Movement events are ALWAYS forwarded to the system; this only controls
// whether they additionally enter the MouseQueue and wake the engine.
#define IOCTL_ANYKEY_SET_MOUSE_MOVE   CTL_CODE(FILE_DEVICE_KEYBOARD,  \
                                               ANYKEY_IOCTL_INDEX + 11, \
                                               METHOD_BUFFERED,         \
                                               FILE_WRITE_DATA)

// IOCTL_ANYKEY_SET_CAPTURE -- Enable/disable passthrough capture (passive identify).
// When enabled, the filter mirrors keyboard/mouse input into its queues WITHOUT
// consuming it, so a user-mode client (e.g. the GUI "识别" feature) can poll
// WAIT_INPUT / WAIT_MOUSE_INPUT while keystrokes/mouse still reach the OS.
// Input: BOOLEAN (TRUE = capture, FALSE = off)
#define IOCTL_ANYKEY_SET_CAPTURE      CTL_CODE(FILE_DEVICE_KEYBOARD,  \
                                               ANYKEY_IOCTL_INDEX + 12, \
                                               METHOD_BUFFERED,         \
                                               FILE_WRITE_DATA)

// -- Data structures --

// Input event: driver -> user mode (via IOCTL_ANYKEY_WAIT_INPUT)
typedef struct _ANYKEY_INPUT_EVENT {
    USHORT  MakeCode;       // Scan code (PS/2 Set 1)
    USHORT  Flags;          // KEY_MAKE(0)/KEY_BREAK(1)/KEY_E0(2)/KEY_E1(4)
    ULONG   DeviceId;       // Device index (matches GetDeviceInfo)
    ULONG   ExtraInfo;      // Reserved
} ANYKEY_INPUT_EVENT, *PANYKEY_INPUT_EVENT;

// Output event: user mode -> driver (via IOCTL_ANYKEY_SEND_OUTPUT)
typedef struct _ANYKEY_OUTPUT_EVENT {
    USHORT  MakeCode;       // Scan code to inject
    USHORT  Flags;          // KEY_MAKE(0)/KEY_BREAK(1)/KEY_E0(2)/KEY_E1(4)
    ULONG   DeviceId;       // Target device (0 = default keyboard, 1+ = specific)
} ANYKEY_OUTPUT_EVENT, *PANYKEY_OUTPUT_EVENT;

// Intercept request: user mode -> driver (via IOCTL_ANYKEY_SET_INTERCEPT)
// DeviceId=0 applies to ALL devices; Enable sets per-device InterceptEnabled.
typedef struct _ANYKEY_INTERCEPT_REQUEST {
    ULONG   DeviceId;       // 0 = all devices, 1+ = specific device
    BOOLEAN Enable;         // TRUE = intercept, FALSE = pass-through
} ANYKEY_INTERCEPT_REQUEST, *PANYKEY_INTERCEPT_REQUEST;

// -- Mouse event structures (v0.2) --

// Mouse input event: driver -> user mode (via IOCTL_ANYKEY_WAIT_MOUSE_INPUT)
// Mirrors MOUSE_INPUT_DATA from ntddmou.h with DeviceId prepended.
typedef struct _ANYKEY_MOUSE_EVENT {
    ULONG   DeviceId;       // Which mouse device
    USHORT  Flags;          // MOUSE_MOVE_RELATIVE(0) / MOUSE_MOVE_ABSOLUTE(1)
                            // MOUSE_VIRTUAL_DESKTOP(2) / MOUSE_ATTRIBUTES_CHANGED(4)
    USHORT  ButtonFlags;    // Combination of MOUSE_LEFT/RIGHT/MIDDLE_BUTTON_DOWN/UP
                            // MOUSE_WHEEL / MOUSE_HWHEEL (when wheel rotation)
                            // MOUSE_BUTTON_4_DOWN/UP (XButton1)
                            // MOUSE_BUTTON_5_DOWN/UP (XButton2)
    SHORT   ButtonData;     // Wheel delta: +120 = forward, -120 = backward
                            // (WHEEL_DELTA = 120 for standard mice)
    LONG    LastX;          // Relative movement X, or absolute coordinate X
    LONG    LastY;          // Relative movement Y, or absolute coordinate Y
    ULONG   ExtraInfo;      // Original MOUSE_INPUT_DATA.ExtraInformation
} ANYKEY_MOUSE_EVENT, *PANYKEY_MOUSE_EVENT;

// Mouse output event: user mode -> driver (via IOCTL_ANYKEY_SEND_MOUSE_OUTPUT)
typedef struct _ANYKEY_MOUSE_OUTPUT_EVENT {
    ULONG   DeviceId;       // Target device (0 = default mouse, 1+ = specific)
    USHORT  Flags;          // MOUSE_MOVE_RELATIVE (typically)
    USHORT  ButtonFlags;    // Button down/up/wheel flags to inject
    SHORT   ButtonData;     // Wheel delta
    LONG    LastX;          // Relative movement X
    LONG    LastY;          // Relative movement Y
} ANYKEY_MOUSE_OUTPUT_EVENT, *PANYKEY_MOUSE_OUTPUT_EVENT;

// -- Device type constants for DEVICE_EXTENSION.DeviceType --
#define ANYKEY_DEV_KEYBOARD   0
#define ANYKEY_DEV_MOUSE      1

// Device info: driver -> user mode (via IOCTL_ANYKEY_GET_DEVICE_INFO)
typedef struct _ANYKEY_DEVICE_INFO {
    ULONG   DeviceId;           // Device index
    WCHAR   HardwareId[128];    // Hardware ID string (e.g., "HID\VID_046D&PID_...")
    WCHAR   ContainerId[40];    // ContainerID GUID string (empty if none)
    WCHAR   FriendlyName[64];   // PnP device description for GUI display
    USHORT  VendorId;            // Numeric VID (parsed from HardwareId)
    USHORT  ProductId;           // Numeric PID
    BOOLEAN IsKeyboard;          // TRUE for keyboard, FALSE for mouse
    BOOLEAN IsMouse;             // TRUE for mouse (driver doesn't intercept yet)
    BOOLEAN HasSerialNumber;     // USB iSerialNumber != 0 -> ContainerID stable
    ULONG   Flags;               // ANYKEY_DEV_FLAG_PS2 / _NO_SN / etc.
} ANYKEY_DEVICE_INFO, *PANYKEY_DEVICE_INFO;

// -- Device flags --
#define ANYKEY_DEV_FLAG_PS2          0x00000001
#define ANYKEY_DEV_FLAG_NO_SERIAL    0x00000002
#define ANYKEY_DEV_FLAG_VIRTUAL      0x00000004  // hyperkbd, terminpt, etc.
#define ANYKEY_DEV_FLAG_MOUSE        0x00000008  // Device is a mouse (not keyboard)

// -- Key state flags (matching PS/2 Set 1 convention) --
#define ANYKEY_KEY_MAKE     0x00
#define ANYKEY_KEY_BREAK    0x01
#define ANYKEY_KEY_E0       0x02
#define ANYKEY_KEY_E1       0x04

// -- Enum devices request (IOCTL_ANYKEY_ENUM_DEVICES) --
typedef struct _ANYKEY_ENUM_DEVICES_REQUEST {
    ULONG Index;       // Start index (0-based)
    ULONG MaxCount;    // Max devices to return
} ANYKEY_ENUM_DEVICES_REQUEST, *PANYKEY_ENUM_DEVICES_REQUEST;

// -- Driver status (IOCTL_ANYKEY_GET_STATUS) --
#define ANYKEY_FLAG_DEVICE_CHANGED  0x01

typedef struct _ANYKEY_DRIVER_STATUS {
    ULONG   DeviceCount;
    ULONG   Flags;              // ANYKEY_FLAG_DEVICE_CHANGED, etc.
    ULONG   InterceptingCount;  // how many devices currently have InterceptEnabled=TRUE
} ANYKEY_DRIVER_STATUS, *PANYKEY_DRIVER_STATUS;

// -- Heartbeat response (IOCTL_ANYKEY_HEARTBEAT output) --
#define ANYKEY_DRIVER_VERSION  0x00040000  // major.minor (v0.4: per-device intercept)
typedef struct _ANYKEY_HEARTBEAT_RESPONSE {
    ULONG           DriverVersion;      // ANYKEY_DRIVER_VERSION
    ULONG           QueueDepth;         // Total queued events across all devices
    ULONG           DeviceCount;        // Number of filter devices
    LARGE_INTEGER   Timestamp;          // KeQueryInterruptTime at last heartbeat
    ULONG           StateFlags;         // 0=healthy, see ANYKEY_STATE_*
} ANYKEY_HEARTBEAT_RESPONSE, *PANYKEY_HEARTBEAT_RESPONSE;

#define ANYKEY_STATE_HEALTHY       0x00
#define ANYKEY_STATE_INTERCEPTING  0x01
#define ANYKEY_STATE_EMERGENCY     0x02

#endif // _ANKEY_FLT_PUBLIC_H
