제가 그 아이디어를 적었을 때는 기술적인 것 보다는 내용적인 면이였습니다.

- 서버가 LLM을 직접 호출하는 라이브러리가 필요한 것은 아닌 것 같고
- 지금의 agent-client는 서버와 LLM 사이에서 프로토콜을 통역해준다 볼 수 있는데,
    - 역할도 기본적으로 1NPC당 1세션입니다
- 일종의 agent 매니저 같은 것을 만들어서
    - 다이나믹하게 온 디맨드로 NPC를 생성
        - 이름, 성격, 성별, 직업, 등등을 LLM에게 만들라고 시킨다 (이게 이 매니저의 시스템 프롬프트)
        - LLM이 지금 instance.txt, merchant.txt 나 guard.txt 같은 캐릭터 설명 텍스트를 작성한다 (혹은 json. 빠진 필드가 없는지 확인하기 쉽게)
        - 세션이 시작되고, agent-client 처럼 캐릭터를 만들고 게임에 진입해서 지금 생성한 NPC를 LLM이 롤 플레잉한다
    - 필요에 따라 NPC를 제거
        - 자연스럽게 마을 밖으로
        - 자연스럽게 던전 밖으로
- 만약에 위의 일회성 agent가 잘 된다면,
    - Karl이나 Rica를 전투 가능으로 만들고
    - 마을에서 유저들과 싸우다 사망하면 (예: 욕설을 하는 유저를 가서 두들겨 패다가 전투가 시작되어서 사망)
    - Karl 역할을 하는 새 NPC를 생성 (이름, 배경, 성장 과정 등등을 LLM 보고 쓰라고 시킨다.)
        - 2대 (N+1대) NPC로 등장 (예: 저는 전임 경비병 Karl이 사망해서 중앙에서 새로 파견나온 Otto입니다)
    - 던전 보스도 매번 죽을 때마다 새로 생성 (그래봤자 이름과 배경 스토리 바뀌는 정도겠지만...)

---

## English

When I wrote that idea down, I was thinking about it in terms of content rather than technology.

- I don't think we need a library that lets the server call an LLM directly.
- The current agent-client can be seen as translating the protocol between the server and the LLM.
    - Its role is basically one session per NPC.
- Build something like an agent manager that would:
    - Create NPCs dynamically, on demand.
        - Ask the LLM to invent the name, personality, gender, occupation, and so on (this is the manager's system prompt).
        - The LLM writes the character description text — like today's instance.txt, merchant.txt, or guard.txt (or JSON, so it's easy to check that no field is missing).
        - A session starts, creates a character and enters the game just like agent-client does, and the LLM role-plays the NPC it just generated.
    - Remove NPCs as needed.
        - Have them walk out of town naturally.
        - Have them leave the dungeon naturally.
- If the disposable agents above work well, then:
    - Make Karl and Rica combat-capable.
    - If they fight users in town and die (e.g. they go beat up a user who is swearing, combat starts, and they get killed),
    - Generate a new NPC to take over Karl's role (have the LLM write the name, background, upbringing, etc.).
        - It shows up as the 2nd-generation (N+1th) NPC — e.g. "The previous guard, Karl, was killed, so I'm Otto, newly dispatched from the capital."
    - Regenerate the dungeon boss every time it dies too (though that probably amounts to little more than a new name and backstory...).

