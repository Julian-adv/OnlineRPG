# Party Icons

시안 A의 클래스·파티장 표시를 실제 HUD에서 사용할 수 있도록 만든 투명 배경 SVG 리소스다. 모든 아이콘은 `24 × 24` viewBox를 사용한다.

- `class-knight.svg`: 나이트용 방패와 검
- `class-barbarian.svg`: 바바리안용 교차 도끼
- `class-caveman.svg`: 케이브맨용 가시 곤봉
- `class-valkyrie.svg`: 발키리용 날개 투구
- `class-ranger.svg`: 레인저용 활과 화살
- `class-priest.svg`: 프리스트용 십자가
- `class-rogue.svg`: 로그용 교차 쌍단검
- `leader-crown.svg`: 클래스와 무관한 파티장 표시

정적 경로는 `/icons/party/<파일명>`이다.

```svelte
<img src="/icons/party/class-knight.svg" alt="Knight" width="18" height="18" />
```
