import { ref } from "vue";

export interface ContextMenuItem {
  label: string;
  action: () => void;
  separator?: boolean;
  icon?: string;
  danger?: boolean;
}

const visible = ref(false);
const x = ref(0);
const y = ref(0);
const items = ref<ContextMenuItem[]>([]);

function show(event: MouseEvent, menuItems: ContextMenuItem[]) {
  items.value = menuItems;

  // Position with viewport clamping
  const menuWidth = 200;
  const menuHeight = menuItems.filter((i) => !i.separator).length * 32 +
    menuItems.filter((i) => i.separator).length * 9 + 8;

  let posX = event.clientX;
  let posY = event.clientY;

  if (posX + menuWidth > window.innerWidth) {
    posX = window.innerWidth - menuWidth - 8;
  }
  if (posY + menuHeight > window.innerHeight) {
    posY = window.innerHeight - menuHeight - 8;
  }
  if (posX < 0) posX = 8;
  if (posY < 0) posY = 8;

  x.value = posX;
  y.value = posY;
  visible.value = true;
}

function close() {
  visible.value = false;
}

export function useContextMenu() {
  return {
    visible,
    x,
    y,
    items,
    show,
    close,
  };
}
