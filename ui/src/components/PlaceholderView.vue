<script setup>
import Icon from './Icon.vue'

const props = defineProps({
  title: { type: String, required: true },
  desc: { type: String, default: '' },
  icon: { type: String, default: 'sparkle' },
  grad: { type: String, default: 'blue' },
})
</script>

<template>
  <div class="wrap">
    <div class="hero" :class="'grad-' + grad">
      <div class="hero-ico"><Icon :name="icon" :size="30" /></div>
      <h2>{{ title }}</h2>
      <p>{{ desc || '这个模块正在接入中，照片库页面已完整实现。' }}</p>
      <div class="chip on-grad">即将上线</div>
    </div>
  </div>
</template>

<style scoped>
.wrap { height: 100%; }
.hero {
  height: 100%;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 12px;
  border-radius: var(--r-xl);
  color: #fff;
  box-shadow: var(--shadow-lg);
  position: relative;
  overflow: hidden;
}
.hero::after {
  content: '';
  position: absolute; inset: 0;
  background: radial-gradient(90% 70% at 50% 0%, rgba(255,255,255,.22), transparent 70%);
  pointer-events: none;
}
.grad-blue   { background: var(--grad-blue); }
.grad-violet { background: var(--grad-violet); }
.grad-pink   { background: var(--grad-pink); }
.grad-green  { background: var(--grad-green); }
.grad-amber  { background: var(--grad-amber); }
.hero-ico {
  display: grid; place-items: center;
  width: 68px; height: 68px;
  border-radius: var(--r-lg);
  background: rgba(255,255,255,.2);
  backdrop-filter: blur(4px);
  z-index: 1;
}
.hero h2 { margin: 0; font-family: var(--font-display); font-size: var(--fs-2xl); z-index: 1; }
.hero p { margin: 0; font-size: var(--fs-md); color: rgba(255,255,255,.85); z-index: 1; }
.chip {
  margin-top: 8px;
  padding: 7px 18px;
  border-radius: var(--r-pill);
  background: rgba(255,255,255,.22);
  font-size: var(--fs-sm);
  font-weight: 600;
  z-index: 1;
}
</style>
