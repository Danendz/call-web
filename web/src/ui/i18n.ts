import { createContext, useContext } from "react";
import type { Lang } from "../../../shared/protocol";
import type { MediaAccess } from "../call/media";

// The interface speaks the language picked in "I speak", so each person reads it in the language they talk in.

const en = {
  roomFull: "Room is full",
  roomFullBody: "This room already has two people in it.",
  startNewRoom: "Start a new room",
  callEnded: "Call ended",
  rejoin: "Rejoin",
  newRoom: "New room",

  roomLink: "Room link",
  copy: "Copy",
  copied: "Copied",

  readyToJoin: "Ready to join?",
  shareLink: "Share this link with the person you want to call",
  microphone: "Microphone",
  camera: "Camera",
  defaultDevice: "Default",
  iSpeak: "I speak",
  cameraOff: "Camera is off",
  access: {
    granted: "",
    pending: "Asking for permission…",
    denied: "Blocked by the browser",
    missing: "Not found",
    busy: "In use by another app",
  } satisfies Record<MediaAccess, string>,
  deviceStatus: (device: string, status: string) => `${device}: ${status}`,
  permissionHelp:
    "The browser blocked access. Tap the camera or lock icon in the address bar, allow camera and microphone, then press Retry. On iPhone: Settings → Safari → Camera / Microphone → Allow.",
  retry: "Retry",
  waitingPermission: "Waiting for permission…",
  joinWithoutMedia: "Join without camera & mic",
  joinCall: "Join call",

  micOn: "Turn on microphone",
  micOff: "Turn off microphone",
  camOn: "Turn on camera",
  camOff: "Turn off camera",
  sendPortrait: "Send portrait video",
  sendLandscape: "Send landscape video",
  portraitVideo: "Portrait video",
  speechToChat: "Speech to chat",
  shareScreen: "Share screen",
  stopSharing: "Stop sharing",
  stats: "Connection stats",
  chat: "Chat",
  more: "More",
  hangUp: "Hang up",
  swapVideos: "Swap videos",

  starting: "Starting…",
  waitingForPeer: "Waiting for the other person to join…",
  serverLost: "Lost connection to server, reconnecting…",
  connecting: "Connecting…",
  unstable: "Connection unstable, reconnecting…",
  failed: "Connection failed, retrying…",
  speechUnavailable: "Speech-to-chat isn't available in this browser",
  speechPaused: (at: string) => `Speech-to-chat paused until ${at}: free limit reached`,

  closeChat: "Close chat",
  noMessages: "No messages yet. They disappear when the room closes.",
  spoken: "Spoken",
  message: "Message",
  send: "Send",
};

export type Strings = typeof en;

const zh: Strings = {
  roomFull: "房间已满",
  roomFullBody: "这个房间已经有两个人了。",
  startNewRoom: "创建新房间",
  callEnded: "通话已结束",
  rejoin: "重新加入",
  newRoom: "新房间",

  roomLink: "房间链接",
  copy: "复制",
  copied: "已复制",

  readyToJoin: "准备好加入了吗？",
  shareLink: "把这个链接发给你要通话的人",
  microphone: "麦克风",
  camera: "摄像头",
  defaultDevice: "默认",
  iSpeak: "我说",
  cameraOff: "摄像头已关闭",
  access: {
    granted: "",
    pending: "正在请求授权…",
    denied: "被浏览器阻止",
    missing: "未找到",
    busy: "正被其他应用占用",
  },
  deviceStatus: (device, status) => `${device}：${status}`,
  permissionHelp:
    "浏览器阻止了访问。点击地址栏中的摄像头或锁形图标，允许使用摄像头和麦克风，然后点击“重试”。iPhone：设置 → Safari → 摄像头 / 麦克风 → 允许。",
  retry: "重试",
  waitingPermission: "等待授权…",
  joinWithoutMedia: "不开摄像头和麦克风加入",
  joinCall: "加入通话",

  micOn: "打开麦克风",
  micOff: "关闭麦克风",
  camOn: "打开摄像头",
  camOff: "关闭摄像头",
  sendPortrait: "发送竖屏视频",
  sendLandscape: "发送横屏视频",
  portraitVideo: "竖屏视频",
  speechToChat: "语音转文字",
  shareScreen: "共享屏幕",
  stopSharing: "停止共享",
  stats: "连接统计",
  chat: "聊天",
  more: "更多",
  hangUp: "挂断",
  swapVideos: "切换画面",

  starting: "正在启动…",
  waitingForPeer: "等待对方加入…",
  serverLost: "与服务器断开连接，正在重连…",
  connecting: "正在连接…",
  unstable: "连接不稳定，正在重连…",
  failed: "连接失败，正在重试…",
  speechUnavailable: "此浏览器不支持语音转文字",
  speechPaused: (at) => `语音转文字已暂停至 ${at}：已达到免费额度`,

  closeChat: "关闭聊天",
  noMessages: "还没有消息。房间关闭后消息会消失。",
  spoken: "语音",
  message: "消息",
  send: "发送",
};

export const STRINGS: Record<Lang, Strings> = { en, zh };

export const LangContext = createContext<Lang>("en");

export function useT(): Strings {
  return STRINGS[useContext(LangContext)];
}
