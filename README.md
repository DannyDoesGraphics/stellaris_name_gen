# Stellaris Name List Generator
Are you too lazy to properly generate name lists for your stellaris game?
Do you have Ollama, Gemini, ChatGPT, or [insert LLM provider here]?
Well we have a solution for you!

A stellaris name list generator will automatically generate a name list configuration with a matching localisation using your given story prompt.


## How?
- We expect a `file_structure.txt` containing your base [stellaris namelist](https://stellaris.paradoxwikis.com/Empire_modding#Name_lists)
- `lore.txt` contains your setting to set your prompt in ie if you're a 40k empire, put this information here! The LLM will use this information to inform itself how to generate suitable names
- We expect `.env` file containing **one** of your API key that you will use:
    - Gemini: `GEMINI_API_KEY`
    - OpenAI: `OPENAI_API_KEY`
- We optionally add a `localisation_base.yml` based off of existing [localisation](https://stellaris.paradoxwikis.com/Localisation_modding)
- **Note:** The best to learn to initially setup your localisation and file structure files is to reference the games' base reference namelists ie HUMAN1.txt (found in game directory common/namelists)

### file_structure.txt
```
NAME = {
    # prefix: prefix_that_will_propogate_down_to_all_descendants_
    character_names = {
        # Prompt to give to your LLM to generate name1's table
        # weight = 50
        name1 = {
            
        }
        
        name2 = {
            weight = 50 # Optionally, you can define weight as it's own parameter in here too
            MY_LOCALISATION_KEY # You as well can define your own localisation keys if you have a pre-existing setup. This will however mean that the generator **will not generate this table**
        }
    }
}
```
---

Once your file_structure.txt is setup, you can install rust and run:
```
cargo run
```
and leave your PC for a moment whilst it generates everything for you.

## Structure
- `out/` directory is effectively a cache. Delete this directory if you want to re-run your LLM


## Output?
- `out.txt` contains the name list configuration
- `localisation.yml` contains the localisation for the name list