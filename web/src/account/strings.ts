import { useContext } from "react";
import type { ApiErrorCode } from "../../../shared/api";
import type { Lang } from "../../../shared/protocol";
import { LangContext } from "../ui/i18n";

const en = {
  signIn: "Sign in",
  createAccount: "Create account",
  forgotPassword: "Forgot password",
  username: "Username",
  password: "Password",
  newPassword: "New password",
  currentPassword: "Current password",
  inviteCode: "Invite code",
  recoveryCode: "Recovery code",
  usernameRule: "3–20 lowercase letters, digits or _",
  passwordRule: "At least 10 characters",
  inviteOnly: "Accounts are by invite: ask someone who has one for a code.",
  recoverHelp: "Enter the recovery code you saved when you created the account.",
  saveRecoveryTitle: "Save your recovery code",
  saveRecoveryBody: "It's the only way back in if you forget your password, and it's shown only now.",
  savedIt: "I saved it",
  withoutAccount: "Use a room link without an account",

  account: "Account",
  signOut: "Sign out",
  changePassword: "Change password",
  passwordChanged: "Password changed. Your other devices were signed out.",
  invites: "Invite codes",
  invitesBody: (unused: number, max: number) => `Each code makes one account and works for 7 days. Unused: ${unused} of ${max}.`,
  newInvite: "New invite code",
  shownOnce: "Copy it now: it's shown only once.",
  blocked: "Blocked",
  unblock: "Unblock",
  deleteAccount: "Delete account",
  deleteBody: "Deletes your account, friends and missed calls for good. Enter your password to confirm.",
  language: "Language",

  errors: {
    "bad-request": "Something's wrong with that request.",
    "not-found": "Something's wrong with that request.",
    unauthorized: "You were signed out. Sign in again.",
    forbidden: "Not allowed.",
    "rate-limited": "Too many tries. Wait a while and try again.",
    unavailable: "Can't reach the server. Try again.",
    "invalid-credentials": "Wrong username or password.",
    "invalid-invite": "That invite code isn't valid, or was already used.",
    "invalid-recovery": "Wrong username or recovery code.",
    "username-taken": "That username is taken.",
    "invalid-username": "Usernames are 3–20 lowercase letters, digits or _.",
    "weak-password": "Pick a longer password that's harder to guess (at least 10 characters, not your username).",
    "too-many-invites": "You already have the most unused codes.",
    "too-many": "You've reached the limit for this.",
  } satisfies Record<ApiErrorCode, string>,
};

const zh: typeof en = {
  signIn: "登录",
  createAccount: "注册",
  forgotPassword: "忘记密码",
  username: "用户名",
  password: "密码",
  newPassword: "新密码",
  currentPassword: "当前密码",
  inviteCode: "邀请码",
  recoveryCode: "恢复码",
  usernameRule: "3–20 个小写字母、数字或 _",
  passwordRule: "至少 10 个字符",
  inviteOnly: "注册需要邀请码：请向已有账号的人要一个。",
  recoverHelp: "输入注册时保存的恢复码。",
  saveRecoveryTitle: "保存你的恢复码",
  saveRecoveryBody: "忘记密码时这是唯一的找回方式，而且只显示这一次。",
  savedIt: "我已保存",
  withoutAccount: "不登录，使用房间链接",

  account: "账号",
  signOut: "退出登录",
  changePassword: "修改密码",
  passwordChanged: "密码已修改。你的其他设备已退出登录。",
  invites: "邀请码",
  invitesBody: (unused, max) => `每个邀请码可注册一个账号，7 天内有效。未使用：${unused} / ${max}。`,
  newInvite: "生成邀请码",
  shownOnce: "现在就复制：它只显示这一次。",
  blocked: "已屏蔽",
  unblock: "取消屏蔽",
  deleteAccount: "删除账号",
  deleteBody: "将永久删除你的账号、好友和未接来电。输入密码确认。",
  language: "语言",

  errors: {
    "bad-request": "请求有误。",
    "not-found": "请求有误。",
    unauthorized: "你已退出登录，请重新登录。",
    forbidden: "不允许此操作。",
    "rate-limited": "尝试次数过多，请稍后再试。",
    unavailable: "无法连接服务器，请重试。",
    "invalid-credentials": "用户名或密码错误。",
    "invalid-invite": "邀请码无效或已被使用。",
    "invalid-recovery": "用户名或恢复码错误。",
    "username-taken": "该用户名已被使用。",
    "invalid-username": "用户名为 3–20 个小写字母、数字或 _。",
    "weak-password": "请使用更长、更难猜的密码（至少 10 个字符，且不包含用户名）。",
    "too-many-invites": "未使用的邀请码已达上限。",
    "too-many": "已达到上限。",
  },
};

const STRINGS: Record<Lang, typeof en> = { en, zh };

export function useAccountT(): typeof en {
  return STRINGS[useContext(LangContext)];
}
