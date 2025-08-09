<script lang="ts">
	import { gameStore } from '../stores/gameStore';
	import { networkManager } from '../network/socket';

	let chatMessages: string[] = $state([]);
	let isConnected = $state(false);
	let messageInput = $state('');
	let chatContainer: HTMLDivElement;

	gameStore.subscribe(state => {
		chatMessages = state.chatMessages;
		isConnected = state.isConnected;
	});

	$effect(() => {
		if (chatContainer) {
			chatContainer.scrollTop = chatContainer.scrollHeight;
		}
	});

	function sendMessage() {
		if (messageInput.trim() && isConnected) {
			networkManager.sendChatMessage(messageInput.trim());
			messageInput = '';
		}
	}

	function handleKeyDown(event: KeyboardEvent) {
		if (event.key === 'Enter') {
			event.preventDefault();
			sendMessage();
		}
	}
</script>

<div class="chat-panel">
	<div class="chat-header">
		<h3>Chat</h3>
		<div class="connection-status" class:connected={isConnected}>
			{isConnected ? '🟢' : '🔴'} {isConnected ? 'Connected' : 'Disconnected'}
		</div>
	</div>

	<div class="chat-messages" bind:this={chatContainer}>
		{#each chatMessages as message, index}
			<div class="message">
				{message}
			</div>
		{/each}
	</div>

	<div class="chat-input">
		<input
			type="text"
			bind:value={messageInput}
			onkeydown={handleKeyDown}
			placeholder="Type a message..."
			disabled={!isConnected}
		/>
		<button onclick={sendMessage} disabled={!isConnected || !messageInput.trim()}>
			Send
		</button>
	</div>
</div>

<style>
	.chat-panel {
		position: fixed;
		bottom: 20px;
		left: 20px;
		width: 350px;
		height: 300px;
		background: rgba(0, 0, 0, 0.8);
		border: 1px solid #4a5568;
		border-radius: 8px;
		display: flex;
		flex-direction: column;
		font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
	}

	.chat-header {
		display: flex;
		justify-content: space-between;
		align-items: center;
		padding: 10px 15px;
		border-bottom: 1px solid #4a5568;
		background: rgba(0, 0, 0, 0.9);
		border-radius: 8px 8px 0 0;
	}

	.chat-header h3 {
		margin: 0;
		color: #ffffff;
		font-size: 14px;
		font-weight: 600;
	}

	.connection-status {
		font-size: 12px;
		color: #a0aec0;
		display: flex;
		align-items: center;
		gap: 5px;
	}

	.connection-status.connected {
		color: #68d391;
	}

	.chat-messages {
		flex: 1;
		overflow-y: auto;
		padding: 10px;
		display: flex;
		flex-direction: column;
		gap: 5px;
	}

	.message {
		color: #e2e8f0;
		font-size: 12px;
		line-height: 1.4;
		word-wrap: break-word;
	}

	.chat-input {
		display: flex;
		padding: 10px;
		gap: 8px;
		border-top: 1px solid #4a5568;
		background: rgba(0, 0, 0, 0.9);
		border-radius: 0 0 8px 8px;
	}

	.chat-input input {
		flex: 1;
		padding: 8px 10px;
		border: 1px solid #4a5568;
		border-radius: 4px;
		background: #1a202c;
		color: #ffffff;
		font-size: 12px;
	}

	.chat-input input:focus {
		outline: none;
		border-color: #4299e1;
	}

	.chat-input input:disabled {
		opacity: 0.5;
	}

	.chat-input button {
		padding: 8px 15px;
		border: none;
		border-radius: 4px;
		background: #4299e1;
		color: #ffffff;
		font-size: 12px;
		cursor: pointer;
		transition: background-color 0.2s;
	}

	.chat-input button:hover:not(:disabled) {
		background: #3182ce;
	}

	.chat-input button:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}

	.chat-messages::-webkit-scrollbar {
		width: 6px;
	}

	.chat-messages::-webkit-scrollbar-track {
		background: #2d3748;
	}

	.chat-messages::-webkit-scrollbar-thumb {
		background: #4a5568;
		border-radius: 3px;
	}
</style>