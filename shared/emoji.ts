// The emoji picker's grid, under the quick reactions: the ones people use most, not the whole of Unicode. Each can be
// sent as a reaction, so this is also the server's allowlist. The native app keeps the same list
// (`native/app/ui/call/emoji.slint`, and `crates/protocol`, which the fixtures check); change them together.
export const EMOJI_GROUPS: { icon: string; emoji: string[] }[] = [
  {
    icon: "😀",
    emoji: [
      "😀", "😁", "😂", "🤣", "😊", "😇", "🙂", "😉", "😍", "🥰", "😘", "😋", "😛", "😜", "🤪", "😎",
      "🤩", "🥳", "😏", "😌", "😴", "🤔", "🤨", "😐", "🙄", "😬", "😮", "😲", "🥺", "😢", "😭", "😤",
      "😡", "🤯", "😳", "😱", "🤗", "🤭", "🫡", "😷",
    ],
  },
  {
    icon: "👍",
    emoji: [
      "👍", "👎", "👌", "✌️", "🤞", "🤟", "🤙", "👋", "👏", "🙌", "🙏", "💪", "🫶", "👀", "🤝", "✋",
      "👉", "👈", "☝️", "🤷",
    ],
  },
  {
    icon: "❤️",
    emoji: [
      "❤️", "🧡", "💛", "💚", "💙", "💜", "🖤", "🤍", "💔", "💕", "💖", "💘", "💋", "✨", "💯", "🔥",
      "⭐", "💫", "💥", "💤",
    ],
  },
  {
    icon: "🐱",
    emoji: [
      "🐱", "🐶", "🐼", "🐻", "🐰", "🦊", "🐨", "🐯", "🐷", "🐸", "🐧", "🐥", "🦄", "🐝", "🦋", "🌸",
      "🌹", "🌻", "🌈", "☀️", "🌙", "❄️",
    ],
  },
  {
    icon: "🍕",
    emoji: [
      "🍕", "🍔", "🍟", "🍜", "🍣", "🥟", "🍚", "🍰", "🎂", "🍩", "🍪", "🍫", "🍓", "🍉", "🍑", "🥑",
      "☕", "🧋", "🍵", "🍺", "🍷", "🥂",
    ],
  },
  {
    icon: "🎉",
    emoji: [
      "🎉", "🎁", "🎈", "🎮", "🎧", "🎵", "📷", "💻", "📱", "✈️", "🚗", "🏠", "⚽", "🏀", "🏆", "💰",
      "⏰", "📅", "✅", "❌", "❓", "❗",
    ],
  },
];
