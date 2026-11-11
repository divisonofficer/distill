<script lang="ts">
import Markdown from '$lib/components/Markdown.svelte';
import type { ClientMsg, InteractionRequest, Question } from '$lib/types';

let { request, connected, reply }: {
	request: InteractionRequest;
	connected: boolean;
	reply: (msg: ClientMsg) => boolean;
} = $props();

let selected = $state<Record<string, string[]>>({});
let freeText = $state<Record<string, string>>({});
let feedback = $state('');

const answers = $derived.by(() => {
	if (request.t !== 'question.request') return {};
	return Object.fromEntries(request.questions.map((question) => [question.question, answerFor(question)]));
});
const canSubmit = $derived(
	request.t === 'question.request' && request.questions.every((question) => answers[question.question]?.length > 0)
);

function answerFor(question: Question): string[] {
	const text = (Object.hasOwn(freeText, question.question) ? freeText[question.question] : undefined)?.trim();
	const choices = Object.hasOwn(selected, question.question) ? selected[question.question] : [];
	if (!question.multiSelect && text) return [text];
	return [...choices, ...(text && !choices.includes(text) ? [text] : [])];
}

function choose(question: Question, label: string) {
	if (question.multiSelect) {
		const current = Object.hasOwn(selected, question.question) ? selected[question.question] : [];
		selected[question.question] = current.includes(label)
			? current.filter((value) => value !== label)
			: [...current, label];
	} else {
		selected = { ...selected, [question.question]: [label] };
		freeText = { ...freeText, [question.question]: '' };
	}
}

function typeAnswer(question: Question, value: string) {
	freeText = { ...freeText, [question.question]: value };
	if (!question.multiSelect && value.trim()) selected = { ...selected, [question.question]: [] };
}

function cancelQuestion() {
	if (request.t !== 'question.request') return;
	reply({
		t: 'question_reply', id: request.id, session_id: request.session_id,
		tool_call_id: request.tool_call_id, outcome: 'cancelled'
	});
}

function submitQuestion() {
	if (request.t !== 'question.request' || !canSubmit) return;
	reply({
		t: 'question_reply', id: request.id, session_id: request.session_id,
		tool_call_id: request.tool_call_id, outcome: 'accepted', answers
	});
}

function planReply(outcome: 'approved' | 'cancelled' | 'abandoned') {
	if (request.t !== 'plan.request') return;
	const msg: ClientMsg = {
		t: 'plan_reply', id: request.id, session_id: request.session_id,
		tool_call_id: request.tool_call_id, outcome,
		...(outcome === 'cancelled' ? { feedback: feedback.trim() } : {})
	};
	reply(msg);
}
</script>

{#if request.t === 'question.request'}
<section class="rounded-2xl border border-sky-400 bg-sky-50 p-4 shadow-lg dark:border-sky-700 dark:bg-sky-950" aria-label="답변이 필요한 질문">
	<h2 class="mb-3 text-sm font-semibold">질문에 답해주세요</h2>
	{#if request.preview}
		<div class="mb-4 rounded-xl bg-white/70 p-3 dark:bg-neutral-900"><Markdown text={request.preview} /></div>
	{/if}
	<div class="flex flex-col gap-4">
		{#each request.questions as question (question.question)}
			<fieldset class="min-w-0">
				<legend class="mb-2 text-sm font-medium">{question.question}</legend>
				{#if question.options.length}
					<div class="flex flex-col gap-2">
						{#each question.options as option (option.label)}
							<label class="flex min-h-10 cursor-pointer items-start gap-3 rounded-xl border border-neutral-300 bg-white px-3 py-2 dark:border-neutral-700 dark:bg-neutral-900">
								<input
									type={question.multiSelect ? 'checkbox' : 'radio'}
									name={`question-${request.id}-${question.question}`}
									checked={Object.hasOwn(selected, question.question) && selected[question.question].includes(option.label)}
									onchange={() => choose(question, option.label)}
									class="mt-1 accent-neutral-900 dark:accent-neutral-100"
								/>
								<span class="min-w-0"><span class="block text-sm font-medium">{option.label}</span>
									{#if option.description}<span class="block text-xs text-neutral-500">{option.description}</span>{/if}
								</span>
							</label>
						{/each}
					</div>
				{/if}
				<label class="mt-2 block text-xs text-neutral-600 dark:text-neutral-300" for={`free-${request.id}-${question.question}`}>
					기타 답변
				</label>
				<input
					id={`free-${request.id}-${question.question}`}
					value={Object.hasOwn(freeText, question.question) ? freeText[question.question] : ''}
					oninput={(event) => typeAnswer(question, event.currentTarget.value)}
					class="mt-1 min-h-10 w-full rounded-xl border border-neutral-300 bg-white px-3 text-sm dark:border-neutral-700 dark:bg-neutral-900"
					placeholder="직접 입력"
				/>
			</fieldset>
		{/each}
	</div>
	<div class="mt-4 flex gap-2">
		<button type="button" disabled={!connected} class="min-h-10 rounded-xl border border-neutral-300 px-4 text-sm disabled:opacity-40 dark:border-neutral-700" onclick={cancelQuestion}>취소</button>
		<button type="button" disabled={!connected || !canSubmit} class="min-h-10 flex-1 rounded-xl bg-neutral-900 px-4 text-sm font-medium text-white disabled:opacity-40 dark:bg-neutral-100 dark:text-neutral-900" onclick={submitQuestion}>답변 제출</button>
	</div>
</section>
{:else}
<section class="rounded-2xl border border-violet-400 bg-violet-50 p-4 shadow-lg dark:border-violet-700 dark:bg-violet-950" aria-label="계획 승인">
	<h2 class="mb-3 text-sm font-semibold">계획을 검토해주세요</h2>
	{#if request.plan_content}
		<div class="mb-4 rounded-xl bg-white/70 p-3 dark:bg-neutral-900"><Markdown text={request.plan_content} /></div>
	{/if}
	<div class="flex flex-wrap gap-2">
		<button type="button" disabled={!connected} class="min-h-10 flex-1 rounded-xl bg-neutral-900 px-4 text-sm font-medium text-white disabled:opacity-40 dark:bg-neutral-100 dark:text-neutral-900" onclick={() => planReply('approved')}>승인</button>
		<button type="button" disabled={!connected || !feedback.trim()} class="min-h-10 flex-1 rounded-xl border border-neutral-300 px-4 text-sm disabled:opacity-40 dark:border-neutral-700" onclick={() => planReply('cancelled')}>수정 요청</button>
		<button type="button" disabled={!connected} class="min-h-10 rounded-xl border border-neutral-300 px-4 text-sm disabled:opacity-40 dark:border-neutral-700" onclick={() => planReply('abandoned')}>중단</button>
	</div>
	<label class="mt-3 block text-xs text-neutral-600 dark:text-neutral-300" for={`feedback-${request.id}`}>수정 의견</label>
	<textarea id={`feedback-${request.id}`} bind:value={feedback} rows="2" class="mt-1 w-full resize-y rounded-xl border border-neutral-300 bg-white p-3 text-sm dark:border-neutral-700 dark:bg-neutral-900"></textarea>
</section>
{/if}
