import { useContext } from "react";
import type { CallEndReason, Presence, Lang } from "../../../shared/protocol";
import { LangContext } from "../ui/i18n";

const en = {
  friends: "Friends",
  addFriend: "Add a friend by username",
  send: "Send",
  requestSent: "If that username exists, they'll see your request.",
  requests: "Friend requests",
  accept: "Accept",
  decline: "Decline",
  waiting: "Waiting for them to accept",
  withdraw: "Withdraw",
  noFriends: "No friends yet. Add someone by their exact username.",
  presence: { online: "Online", "in-call": "In a call", offline: "Offline" } satisfies Record<Presence, string>,
  call: (name: string) => `Call ${name}`,
  remove: "Remove",
  block: "Block",
  moreFor: (name: string) => `More for ${name}`,
  rooms: "Rooms",
  stillInCall: (name: string) => `${name} is still in the call`,
  rejoin: "Rejoin",
  missedCalls: "Missed calls",
  clear: "Clear",
  newRoomLink: "New room link",
  reconnecting: "Reconnecting…",
  calling: (name: string) => `Calling ${name}…`,
  cancel: "Cancel",
  incoming: (name: string) => `${name} is calling`,
  ended: {
    declined: (name: string) => `${name} declined`,
    busy: (name: string) => `${name} is in another call. They'll see a missed call.`,
    offline: (name: string) => `${name} isn't online. They'll see a missed call.`,
    timeout: (name: string) => `${name} didn't answer`,
    cancelled: () => "",
    unavailable: (name: string) => `Couldn't call ${name}`,
    "answered-elsewhere": () => "",
  } satisfies Record<CallEndReason, (name: string) => string>,
  dismiss: "Dismiss",
};

const zh: typeof en = {
  friends: "好友",
  addFriend: "按用户名添加好友",
  send: "发送",
  requestSent: "如果该用户名存在，对方会看到你的请求。",
  requests: "好友请求",
  accept: "接受",
  decline: "拒绝",
  waiting: "等待对方接受",
  withdraw: "撤回",
  noFriends: "还没有好友。输入完整的用户名来添加。",
  presence: { online: "在线", "in-call": "通话中", offline: "离线" },
  call: (name) => `呼叫 ${name}`,
  remove: "删除",
  block: "屏蔽",
  moreFor: (name) => `${name} 的更多操作`,
  rooms: "房间",
  stillInCall: (name) => `${name} 还在通话中`,
  rejoin: "重新加入",
  missedCalls: "未接来电",
  clear: "清除",
  newRoomLink: "新房间链接",
  reconnecting: "正在重连…",
  calling: (name) => `正在呼叫 ${name}…`,
  cancel: "取消",
  incoming: (name) => `${name} 来电`,
  ended: {
    declined: (name) => `${name} 拒绝了通话`,
    busy: (name) => `${name} 正在通话中。对方会看到未接来电。`,
    offline: (name) => `${name} 不在线。对方会看到未接来电。`,
    timeout: (name) => `${name} 没有接听`,
    cancelled: () => "",
    unavailable: (name) => `无法呼叫 ${name}`,
    "answered-elsewhere": () => "",
  },
  dismiss: "关闭",
};

const STRINGS: Record<Lang, typeof en> = { en, zh };

export function useFriendsT(): typeof en {
  return STRINGS[useContext(LangContext)];
}
