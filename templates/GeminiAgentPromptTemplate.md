# Gemini Agent Prompt Template

/[plan | boost | goal] <1-2 sentence symptom or objective>
- Context/Logs: <file paths or terminal snippet>
- Constraints: <what to omit, change structurally, or not run (e.g., "No QEMU", "Drop .img")>
- Output: <Diagnose only | Update <file>.md plan | Execute Step X>
