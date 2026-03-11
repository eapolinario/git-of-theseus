#!/usr/bin/env bash

git-of-theseus-analyze /subject --outdir /output

git-of-theseus-stack-plot --outfile=/output/stack_plot.png /output/cohorts.json

CMD="git-of-theseus-survival-plot"

if [ "$GOT_SURVIVAL_YEARS" ]; then
  CMD="${CMD} --years=${GOT_SURVIVAL_YEARS}"
fi

if [ "$GOT_SURVIVAL_FIT" ]; then
  CMD="${CMD} --exp-fit"
fi

CMD="${CMD} --outfile=/output/survival_plot.png /output/survival.json"
$CMD
