import type { PermissionOption } from './types';

export function permissionActions(options: PermissionOption[]) {
return {
once: options.find((option) => option.kind === 'allow_once'),
reject: options.find((option) => option.kind === 'reject_once'),
permanent: options.filter((option) => option.kind === 'allow_always' || option.kind === 'reject_always'),
other: options.filter((option) => !['allow_once', 'reject_once', 'allow_always', 'reject_always'].includes(option.kind))
};
}

export function permissionOptionLabel(option: PermissionOption): string {
switch (option.kind) {
case 'allow_once': return '한 번 허용';
case 'reject_once': return '거부';
case 'allow_always': return '다시는 묻지 않기 · 항상 허용';
case 'reject_always': return '항상 거부';
default: return option.name === 'No' ? '거부' : option.name;
}
}

export function permissionPromptState(permissionCount: number, interactionCount: number): boolean {
return permissionCount + interactionCount > 0;
}
